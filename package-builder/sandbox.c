/*
 * The Flox "virtual sandbox" warns or aborts when encountering an ELF access
 * from outside the closure of packages implied by $FLOX_ENV. In this regard
 * it can provide the same guarantees at an ELF level provided by the sandbox
 * itself, but at an _advisory_ level, so that developers are informed of
 * missing dependencies without actually breaking anything.
 *
 * The virtual sandbox is enabled with `FLOX_VIRTUAL_SANDBOX=(warn|enforce)`
 * set in the environment, and we do this when wrapping files in the bin
 * directory in the course of performing a manifest build.
 *
 * As with the parsing of FLOX_ENV_DIRS, it is essential that this parsing
 * of the closure be performant and initialized only once per invocation, so we
 * start by reading closure paths into a btable from $FLOX_ENV/requisites.txt.
 */

#define _GNU_SOURCE
#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <fnmatch.h>
#include <limits.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

// Declare version bindings to work with minimum supported GLIBC versions.
#ifdef linux
#include "glibc-bindings.h"
#endif

// For access to the in_closure() function.
#include "closure.h"

// Audit level derived from FLOX_VIRTUAL_SANDBOX:
//   -1 = not yet initialized, 0 = off, 1 = warn, 2 = enforce, 3 = pure.
// Written exactly once (under init_once, via ensure_init) and only read
// afterwards, so it needs no further synchronization.
int sandbox_level = -1;

// One-time initialization guard.
//
// Initialization reads several environment variables and, on Linux, resolves
// the real open()/openat() via dlsym(). It must run exactly once even if many
// threads make their first intercepted call simultaneously. We funnel it
// through pthread_once() instead of the old racy
// `if (sandbox_level < 0) sandbox_init();` check-then-set.
static pthread_once_t init_once = PTHREAD_ONCE_INIT;

// Per-thread re-entrancy guard.
//
// Our policy checks call libc functions (fopen() of requisites.txt,
// realpath(), and on Linux dlsym()) that themselves open files. Because this
// library interposes open()/openat() process-wide, those internal opens would
// otherwise re-enter our interceptors — and since initialization runs under
// pthread_once(), re-entry on the same thread is an outright deadlock, not
// just wasted work. While this flag is set, the interceptors hand straight off
// to the real function and perform no checking. It is thread-local so one
// thread being "inside" the sandbox never disables checking on another.
static __thread int in_sandbox = 0;

// Per-thread flag marking the current check as a readlinkat(). Reading a
// symlink is metadata access ("looking around") rather than reading
// out-of-closure file contents, so — like a directory listing — it is permitted
// but warned even under enforce. The readlinkat interceptors set this around
// their sandbox_check_path() call; sandbox_check_path() consults it.
static __thread int in_readlink = 0;

// Pointers to the original libc functions (Linux only). On macOS the real
// functions are reached by calling open()/openat() directly: dyld
// interposition deliberately does not redirect references made from within the
// library that defines the interposers, so a self-call lands on libc.
#ifdef linux
static int (*orig_open)(const char *pathname, int flags, ...) = NULL;
static int (*orig_openat)(int dirfd, const char *pathname, int flags,
                          ...) = NULL;
static FILE *(*orig_fopen)(const char *pathname, const char *mode) = NULL;
static FILE *(*orig_fopen64)(const char *pathname, const char *mode) = NULL;
static ssize_t (*orig_readlinkat)(int dirfd, const char *pathname, char *buf,
                                  size_t bufsiz) = NULL;
static ssize_t (*orig_readlink)(const char *pathname, char *buf,
                                size_t bufsiz) = NULL;
// __readlink_chk / __readlinkat_chk are the _FORTIFY_SOURCE=2 variants of
// readlink/readlinkat. Binaries compiled with fortification (e.g. coreutils)
// bind to these names instead of the plain ones, so intercepting only
// readlink/readlinkat misses them entirely.
static ssize_t (*orig_readlink_chk)(const char *pathname, char *buf,
                                    size_t bufsiz, size_t buflen) = NULL;
static ssize_t (*orig_readlinkat_chk)(int dirfd, const char *pathname,
                                      char *buf, size_t bufsiz,
                                      size_t buflen) = NULL;
#endif

// Helper macros for printing debug, warnings, and errors. Each multi-statement
// macro is wrapped in `do { ... } while (0)` so it behaves as a single
// statement when used as the body of an `if`/`else` (the earlier bare-`if`
// forms could silently capture a trailing `else`).
static int debug_sandbox = 0;
// warn_once's "further warnings suppressed" guard. warn_once is currently only
// reached from sandbox_init() (under pthread_once, single-threaded), so this is
// not a live race today; it is atomic for consistency with home_dotfile_hint
// and to stay correct should warn_once ever be called from a threaded path.
static atomic_int warn_count = 0;
#define debug(format, ...)                                                     \
  do {                                                                         \
    if (debug_sandbox)                                                         \
      fprintf(stderr, "SANDBOX DEBUG[%d]: " format "\n", getpid(),             \
              __VA_ARGS__);                                                    \
  } while (0)
#define warn(format, ...)                                                      \
  fprintf(stderr, "SANDBOX WARNING[%d]: " format "\n", getpid(), ##__VA_ARGS__)
#define warn_once(format, ...)                                                 \
  do {                                                                         \
    if (debug_sandbox)                                                         \
      warn(format, ##__VA_ARGS__);                                             \
    else if (atomic_fetch_add_explicit(&warn_count, 1,                         \
                                       memory_order_relaxed) == 0)             \
      warn(format " (further warnings suppressed)", ##__VA_ARGS__);            \
  } while (0)
#define _error(format, ...)                                                    \
  fprintf(stderr, "SANDBOX ERROR[%d]: " format "\n", getpid(), ##__VA_ARGS__)
#define hint(format, ...)                                                      \
  fprintf(stderr, "SANDBOX HINT[%d]: " format "\n", getpid(), ##__VA_ARGS__)

// Resolved realpath of the user's $HOME and its length, captured once during
// initialization. Used to recognize "$HOME/.<dotfile>" accesses (see
// is_home_dotfile below). home_real_len == 0 means $HOME was unset or could
// not be resolved, in which case no path is treated as a home dotfile.
static char home_real[PATH_MAX];
static size_t home_real_len = 0;

// Perform various initialization, which includes loading the original
// glibc functions to be wrapped using dlsym().
void sandbox_init() {

  // Debug sandbox library with FLOX_DEBUG_SANDBOX=1.
  debug_sandbox = (getenv("FLOX_DEBUG_SANDBOX") != NULL);

  // Resolve $HOME once so we can recognize user config dotfiles later. Resolve
  // through realpath() because sandbox_check_path() compares against realpaths.
  const char *home = getenv("HOME");
  if (home != NULL && realpath(home, home_real) != NULL)
    home_real_len = strlen(home_real);

  // Derive audit level from FLOX_VIRTUAL_SANDBOX environment variable.
  const char *flox_virtual_sandbox_value = getenv("FLOX_VIRTUAL_SANDBOX");
  if (flox_virtual_sandbox_value == NULL ||
      (strcmp(flox_virtual_sandbox_value, "off") == 0)) {
    sandbox_level = 0;
  } else if (strcmp(flox_virtual_sandbox_value, "warn") == 0) {
    sandbox_level = 1;
  } else if (strcmp(flox_virtual_sandbox_value, "enforce") == 0) {
    sandbox_level = 2;
  } else if (strcmp(flox_virtual_sandbox_value, "pure") == 0) {
    // Pure mode is just like enforce, but invoked within the Nix sandbox.
    sandbox_level = 3;
  } else {
    warn_once(
        "FLOX_VIRTUAL_SANDBOX must be (off|warn|enforce|pure) ... ignoring");
    sandbox_level = 0;
  }
  debug("sandbox_level=%d", sandbox_level);

#ifdef linux
  // Declare new functions to be intercepted here, then add stub
  // functions below.
  orig_open = dlsym(RTLD_NEXT, "open");
  orig_openat = dlsym(RTLD_NEXT, "openat");
  orig_fopen = dlsym(RTLD_NEXT, "fopen");
  orig_fopen64 = dlsym(RTLD_NEXT, "fopen64");
  orig_readlinkat = dlsym(RTLD_NEXT, "readlinkat");
  orig_readlink = dlsym(RTLD_NEXT, "readlink");
  orig_readlink_chk = dlsym(RTLD_NEXT, "__readlink_chk");
  orig_readlinkat_chk = dlsym(RTLD_NEXT, "__readlinkat_chk");
#endif
}

// Run one-time initialization exactly once per process. Every entry point
// calls this before consulting sandbox_level or the original function
// pointers; pthread_once() makes concurrent first calls safe.
static void ensure_init(void) { pthread_once(&init_once, sandbox_init); }

// Accessor method for determining sandbox_level defined as a
// static int in this file.
int get_sandbox_level() {
  ensure_init();
  return sandbox_level;
}

#ifdef linux
bool sandbox_check_argv0() {
  // Resolve into a local (stack) buffer. A shared `static` buffer here would
  // be a cross-thread data race exactly like the closure resolution buffer.
  // Callers (sandbox_check_path) have already run ensure_init(), so
  // sandbox_level is valid by the time we get here.
  char argv0_path[PATH_MAX];
  // Identify the argv[0] realpath from /proc and flag if it's
  // not in the closure.
  // TODO: find way to detect changes in /proc/self/exe rather than
  //       running realpath() on every path access.
  if (realpath("/proc/self/exe", argv0_path) == NULL) {
    _error("sandbox_check_argv0() realpath() failed");
    fflush(stderr);
    // If realpath() failed to set the realpath then explicitly
    // ensure our buffer returns an empty string.
    argv0_path[0] = '\0';
  }
  // The use of certain paths like `/usr/bin/env` path is ubiquitous and
  // hardcoded to an extent that we cannot really expect developers to
  // replace it in code, so we instead allow exceptions for a limited
  // number of these paths.
  // simply let it be an allowed exception.
  //
  // Once requested by way of the la_version() call, we know that all
  // libraries requested by this PID are similarly linked from /usr/bin/env
  // so we can simply give all lookups a free pass.
  if (strcmp(argv0_path, "/usr/bin/env") == 0 ||
      strcmp(argv0_path, "/bin/sh") == 0 ||
      strcmp(argv0_path, "/usr/bin/dash") == 0) {
    debug("%s is a permitted argv0", argv0_path);
    return true;
  } else {
    return false;
  }
}
#else // Darwin
bool sandbox_check_argv0() { return false; }
#endif

// Some paths are derived from allowed basenames.

// Define the maximum number of directories that can be specified in
// the FLOX_SANDBOX_ALLOW_DIRS environment variable. This is a somewhat
// arbitrary limit, but it should be more than enough for most cases.
#define FLOX_SANDBOX_ALLOW_DIRS_MAXENTRIES 256

// Define the maximum length of a directory path in the FLOX_SANDBOX_ALLOW_DIRS
// environment variable. This is also somewhat arbitrary, but it should
// be more than enough for most cases.
#define FLOX_SANDBOX_ALLOW_DIRS_MAXLEN PATH_MAX

// The allow-list of directory prefixes, parsed once from
// FLOX_SANDBOX_ALLOW_DIRS plus a handful of built-in and environment-derived
// entries. After allow_dirs_once has fired, this array is read-only and can be
// scanned from any number of threads without locking.
//
// NOTE: the entries point either into allow_dirs_buf (the tokenized copy of
// the env var) or directly at strings owned by the environment
// (getenv()/string literals). We never free or mutate them after init.
static pthread_once_t allow_dirs_once = PTHREAD_ONCE_INIT;
static int allow_dirs_count = 0;
static char allow_dirs_buf[FLOX_SANDBOX_ALLOW_DIRS_MAXLEN];
// Entries are read-only after init (string literals, getenv() results, or
// tokens inside allow_dirs_buf), so the array holds `const char *`.
static const char *allow_dirs[FLOX_SANDBOX_ALLOW_DIRS_MAXENTRIES];

// Append one entry to allow_dirs[], guarding against overflow. Only called
// from allow_dirs_init (i.e. under allow_dirs_once), so the increment is safe.
static void allow_dirs_push(const char *dir) {
  if (dir == NULL)
    return;
  if (allow_dirs_count >= FLOX_SANDBOX_ALLOW_DIRS_MAXENTRIES) {
    _error("check_allowed_basenames() too many allow dirs, ignoring '%s'", dir);
    fflush(stderr);
    return;
  }
  allow_dirs[allow_dirs_count++] = dir;
}

// One-time initializer for allow_dirs[], invoked via pthread_once(). This
// replaces a lazy build that ran under a never-initialized mutex — a silent
// no-op on macOS, where concurrent first calls could therefore corrupt the
// shared array.
static void allow_dirs_init(void) {
  // Copy FLOX_SANDBOX_ALLOW_DIRS into a writable buffer and tokenize it on
  // spaces, recording a pointer to each entry.
  const char *allow_dirs_env = getenv("FLOX_SANDBOX_ALLOW_DIRS");
  if (allow_dirs_env != NULL) {
    // strlen, not sizeof: the original `sizeof(allow_dirs_env)` measured the
    // pointer (8 bytes), so the length check never did anything.
    if (strlen(allow_dirs_env) >= FLOX_SANDBOX_ALLOW_DIRS_MAXLEN) {
      _error("check_allowed_basenames() FLOX_SANDBOX_ALLOW_DIRS is too long, "
             "truncating to %d characters",
             FLOX_SANDBOX_ALLOW_DIRS_MAXLEN);
      fflush(stderr);
    }
    // strncpy does not NUL-terminate when the source is too long, so reserve
    // the last byte and terminate explicitly.
    strncpy(allow_dirs_buf, allow_dirs_env, sizeof(allow_dirs_buf) - 1);
    allow_dirs_buf[sizeof(allow_dirs_buf) - 1] = '\0';

    char *saveptr = NULL; // strtok_r() context
    char *allow_dir = strtok_r(allow_dirs_buf, " ", &saveptr);
    while (allow_dir != NULL) {
      debug("check_allowed_basenames() allow_dirs[%d] = %s", allow_dirs_count,
            allow_dir);
      allow_dirs_push(allow_dir);
      allow_dir = strtok_r(NULL, " ", &saveptr);
    }
  }

  // A few built-in entries that are always allowed.
  allow_dirs_push("/tmp");
  allow_dirs_push("/dev");
#ifdef linux
  allow_dirs_push("/sys");
  allow_dirs_push("/proc");
#else // Darwin
  allow_dirs_push("/System/Library");
  allow_dirs_push("/usr/share");
  allow_dirs_push("/var/db/timezone");
#endif

  // And a couple inferred from the environment.
  allow_dirs_push(getenv("FLOX_SRC_DIR"));
  allow_dirs_push(getenv("TMPDIR"));
}

bool check_allowed_basenames(const char *pathname) {
  // Build the allow-list exactly once, even under concurrent first calls.
  pthread_once(&allow_dirs_once, allow_dirs_init);

  // Thread id is used only for debug tracing. Capture it as an unsigned 64-bit
  // value so the same %llu format works on both platforms. On Linux we go
  // through syscall(SYS_gettid) rather than the gettid() wrapper: the wrapper
  // was only added in glibc 2.30, so calling it would bind gettid@GLIBC_2.30
  // and raise the library's minimum glibc above the 2.17/2.2.5 target. macOS
  // uses pthread_threadid_np(), which yields a uint64_t.
#ifdef linux
  unsigned long long tid = (unsigned long long)syscall(SYS_gettid);
#else // Darwin
  uint64_t tid_raw = 0;
  pthread_threadid_np(NULL, &tid_raw);
  unsigned long long tid = (unsigned long long)tid_raw;
#endif

  // Scan the (now immutable) allow-list. The comparison buffer is local, so
  // concurrent scans do not interfere.
  char allow_dir_real_path[PATH_MAX];
  for (int i = 0; i < allow_dirs_count; i++) {
    // We were passed a realpath, so resolve each allow dir to a realpath too
    // before comparing. Allow dirs that do not exist are simply skipped.
    if (realpath(allow_dirs[i], allow_dir_real_path) == NULL) {
      debug("check_allowed_basenames(): skipping path '%s', does not exist",
            allow_dirs[i]);
      continue;
    }
    debug("check_allowed_basenames('%s'): tid=%llu, i=%d, comparing to '%s'",
          pathname, tid, i, allow_dir_real_path);
    // A prefix match alone is not enough: it would allow sibling paths that
    // merely share a textual prefix (e.g. an allowed "/tmp" would also match
    // "/tmpfoo"). Require the match to end on a path-component boundary, i.e.
    // pathname continues with '/' (a child) or '\0' (the dir itself).
    size_t allow_len = strlen(allow_dir_real_path);
    if (strncmp(pathname, allow_dir_real_path, allow_len) == 0 &&
        (pathname[allow_len] == '/' || pathname[allow_len] == '\0')) {
      debug("%s is an allowed basename", pathname);
      return true;
    }
  }
  return false;
}

// User-declared allow-list of glob patterns, from FLOX_SANDBOX_ALLOW (a
// space-separated list). These come from the manifest's `build.<name>.
// sandbox-allow` field and let a build read specific out-of-closure paths
// silently. Unlike allow_dirs (prefix match), these are matched with fnmatch(),
// so `*` and `**` work; a leading `~/` is expanded to $HOME.
//
// Parsed once under allow_globs_once; the array is read-only afterwards.
#define FLOX_SANDBOX_ALLOW_MAXENTRIES 256
#define FLOX_SANDBOX_ALLOW_MAXLEN (16 * 1024)
static pthread_once_t allow_globs_once = PTHREAD_ONCE_INIT;
static char allow_globs_buf[FLOX_SANDBOX_ALLOW_MAXLEN];
// Read-only after init (tokens inside allow_globs_buf), so `const char *`.
static const char *allow_globs[FLOX_SANDBOX_ALLOW_MAXENTRIES];
static int allow_globs_count = 0;

static void allow_globs_init(void) {
  const char *env = getenv("FLOX_SANDBOX_ALLOW");
  if (env == NULL)
    return;
  if (strlen(env) >= sizeof(allow_globs_buf)) {
    _error("FLOX_SANDBOX_ALLOW is too long, truncating to %zu characters",
           sizeof(allow_globs_buf) - 1);
    fflush(stderr);
  }
  strncpy(allow_globs_buf, env, sizeof(allow_globs_buf) - 1);
  allow_globs_buf[sizeof(allow_globs_buf) - 1] = '\0';

  char *saveptr = NULL;
  char *pattern = strtok_r(allow_globs_buf, " ", &saveptr);
  while (pattern != NULL) {
    if (allow_globs_count >= FLOX_SANDBOX_ALLOW_MAXENTRIES) {
      _error("FLOX_SANDBOX_ALLOW has too many entries, using the first %d",
             FLOX_SANDBOX_ALLOW_MAXENTRIES);
      fflush(stderr);
      break;
    }
    debug("FLOX_SANDBOX_ALLOW pattern[%d] = %s", allow_globs_count, pattern);
    allow_globs[allow_globs_count++] = pattern;
    pattern = strtok_r(NULL, " ", &saveptr);
  }
}

// Returns true if `real_path` matches any user-declared sandbox-allow glob.
// fnmatch() is called with flag 0 (no FNM_PATHNAME) so `*`/`**` match across
// directory separators, giving simple recursive patterns like "~/.npm/**".
static bool check_allowed_globs(const char *real_path) {
  pthread_once(&allow_globs_once, allow_globs_init);
  for (int i = 0; i < allow_globs_count; i++) {
    const char *pattern = allow_globs[i];
    // Expand a leading "~/" to "$HOME/" (into a local buffer) so manifest
    // entries can be written relative to the user's home.
    char expanded[PATH_MAX];
    if (pattern[0] == '~' && pattern[1] == '/' && home_real_len > 0 &&
        (size_t)snprintf(expanded, sizeof(expanded), "%s%s", home_real,
                         pattern + 1) < sizeof(expanded)) {
      pattern = expanded;
    }
    if (fnmatch(pattern, real_path, 0) == 0) {
      debug("%s matches sandbox-allow pattern '%s'", real_path, allow_globs[i]);
      return true;
    }
  }
  return false;
}

// Returns true if `path` (an already-resolved realpath) is a hidden entry
// under the user's $HOME — i.e. it begins with "<HOME>/.". This matches user
// config dotfiles and dot-directories like ~/.gitconfig, ~/.netrc, and
// ~/.config/..., which build tools (git, npm, curl, ...) routinely read.
//
// Such accesses are NOT reproducible across machines, but we permit them even
// under enforce (with a warning, see sandbox_check_path) so that purity can be
// adopted incrementally: a build can be made closure-clean first, and only the
// final graduation to sandbox = "pure" gives up $HOME entirely.
static bool is_home_dotfile(const char *path) {
  if (home_real_len == 0)
    return false;
  // Require "<HOME>/." exactly: same prefix, then a path separator, then a dot.
  // The separator check prevents a sibling like "/home/userX" from matching
  // "/home/user", and the dot restricts the allowance to hidden entries.
  return strncmp(path, home_real, home_real_len) == 0 &&
         path[home_real_len] == '/' && path[home_real_len + 1] == '.';
}

// Print, at most once per process, a hint explaining that $HOME dotfile access
// is tolerated and how to move toward a stricter build.
static void home_dotfile_hint(void) {
  // Atomic guard: home_dotfile_hint() is called from sandbox_check_path() on
  // many threads concurrently, so a plain `int` read-modify-write would race
  // (two threads could both print, and the increment is itself undefined under
  // a data race). atomic_fetch_add gives us a correct print-at-most-once.
  static atomic_int hinted = 0;
  if (atomic_fetch_add_explicit(&hinted, 1, memory_order_relaxed) != 0)
    return;
  hint(
      "$HOME dotfiles are permitted under 'warn' and 'enforce' so build purity "
      "can be increased incrementally; the final step to full reproducibility "
      "is sandbox = \"pure\". To silence this for a specific path, add it to "
      "the build's 'sandbox-allow' list in the manifest.");
}

// Returns true if `real_path` resolves to a directory. Opening a directory is
// a build tool "looking around" (opendir/traversal, getcwd resolution, etc.)
// rather than consuming out-of-closure file *contents*, so directory accesses
// are permitted even under enforce — with a warning — rather than failing the
// build. stat() is not itself intercepted, so this adds no re-entrancy risk.
static bool is_directory(const char *real_path) {
  // Use opendir() rather than stat(): on glibc >= 2.33 stat() is exported as
  // stat@GLIBC_2.33, which would silently raise this library's minimum glibc.
  // opendir()/closedir() have been at the baseline version since GLIBC_2.2.5,
  // so they keep libsandbox portable to older hosts.
  DIR *dir = opendir(real_path);
  if (dir == NULL)
    return false;
  closedir(dir);
  return true;
}

// De-duplicate warnings by resolved path. Returns true the first time it is
// called for a given `real_path` (so the caller emits its warning) and false on
// every subsequent call for the same path. A build commonly touches the same
// out-of-closure path hundreds of times — e.g. listing a directory repeatedly
// via readdir() — and without this each touch produced an identical warning,
// flooding the log. This collapses them to one message per path.
//
// The recorded set lives for the life of the (per-build) process; it is never
// cleared and is small in practice (a handful of paths). Thread-safe: the
// directory-listing path is only reached for out-of-closure accesses, not the
// hot in-closure path, so the lock is effectively uncontended. The mutex uses
// PTHREAD_MUTEX_INITIALIZER (valid on both platforms), unlike the removed
// zero-initialized mutex.
#define WARNED_PATHS_MAX 1024
static char *warned_paths[WARNED_PATHS_MAX];
static int warned_paths_count = 0;
static pthread_mutex_t warned_paths_lock = PTHREAD_MUTEX_INITIALIZER;

static bool should_warn_for_path(const char *real_path) {
  pthread_mutex_lock(&warned_paths_lock);
  bool first_time = true;
  for (int i = 0; i < warned_paths_count; i++) {
    if (strcmp(warned_paths[i], real_path) == 0) {
      first_time = false;
      break;
    }
  }
  // Record on first sight. If the table is full or strdup() fails, leave
  // first_time true and warn anyway — a repeated warning is safer than silently
  // dropping one.
  if (first_time && warned_paths_count < WARNED_PATHS_MAX) {
    char *copy = strdup(real_path);
    if (copy != NULL)
      warned_paths[warned_paths_count++] = copy;
  }
  pthread_mutex_unlock(&warned_paths_lock);
  return first_time;
}

// Format a path for a user-facing message into `buf`. When the resolved
// realpath differs from the path as opened (relative paths like "..", symlinks,
// etc.) the realpath is appended in parentheses so the message is actionable;
// otherwise just the path is shown.
static void format_path_display(char *buf, size_t buflen, const char *pathname,
                                const char *real_path) {
  if (strcmp(pathname, real_path) == 0)
    snprintf(buf, buflen, "%s", pathname);
  else
    snprintf(buf, buflen, "%s (%s)", pathname, real_path);
}

// Check if path access represents something that may not be reproducible
// on another machine. Any path within the environment's closure is fine,
// but there are also other specific paths and basenames accessed during a
// build that we can similarly rely to be present on any machine.
//
// The challenge here is that some path accesses are discrete while others
// are modal, implying a different handling for subsequent path accesses.
// One example of this is the use of `/usr/bin/env`, which is ubiquitous
// and hardcoded to an extent that we cannot really expect users to replace
// references to it in code, so when invoking this path we suspend all
// further path checking until argv0 is updated to a new path.
bool sandbox_check_path(const char *pathname) {
  ensure_init();
  if (sandbox_level == 0)
    return true;
  debug("sandbox_check_path('%s'), sandbox_level=%d", pathname, sandbox_level);
  if (sandbox_check_argv0())
    return true;

  // From here on out, operate on realpath. If a file doesn't exist then return
  // true and let ENOENT be the eventual result. This must be a local (stack)
  // buffer: a shared `static` here was a data race, since concurrent callers
  // resolving different paths would overwrite each other between this call and
  // the closure/allow-list checks below.
  char real_path[PATH_MAX];
  if (realpath(pathname, real_path) == NULL)
    return true;
  if (check_allowed_basenames(real_path))
    return true;
  // User-declared sandbox-allow patterns are explicit exceptions: allow them
  // silently (no warning), the same as the built-in allow dirs.
  if (check_allowed_globs(real_path))
    return true;
  if (in_closure(real_path)) {
    debug("%s is in the closure", pathname);
    return true;
  }

  // Surface the resolved realpath alongside the opened path in any message
  // below, so relative paths ("..") and symlinks are intelligible.
  char display[PATH_MAX * 2 + 4];
  format_path_display(display, sizeof(display), pathname, real_path);

  // A readlinkat() (in_readlink) is metadata access — "looking around" like a
  // directory listing — rather than reading out-of-closure contents, so permit
  // it even under enforce, with a one-per-path warning.
  if (in_readlink) {
    if (should_warn_for_path(real_path))
      warn("%s is outside the closure but permitted (symlink read)", display);
    return true;
  }

  // Directory accesses are "looking around" rather than reading out-of-closure
  // contents, so permit them even under enforce — with a warning, but only the
  // first time we see each directory (builds list the same directory many
  // times, which otherwise floods the log).
  if (is_directory(real_path)) {
    if (should_warn_for_path(real_path))
      warn("%s is outside the closure but permitted (directory listing)",
           display);
    return true;
  }
  // User config dotfiles under $HOME are permitted even under enforce, but
  // flagged (and followed by a one-time hint), so the developer knows the build
  // still depends on $HOME state on the path to full purity. As with directory
  // listings, warn only the first time we see each dotfile — builds re-read the
  // same config files (~/.gitconfig, ~/.npmrc, ...) repeatedly.
  if (is_home_dotfile(real_path)) {
    if (should_warn_for_path(real_path)) {
      warn("%s is outside the closure but permitted as a $HOME dotfile",
           display);
      home_dotfile_hint();
    }
    return true;
  }
  if (sandbox_level == 1) {
    // warn mode: report the out-of-closure read, but only once per path —
    // a build that reads the same undeclared file repeatedly should produce a
    // single warning, not one per read.
    if (should_warn_for_path(real_path))
      warn("%s is not in the sandbox", display);
    return true;
  }
  // enforce / pure: an out-of-closure file read is fatal.
  _error("%s is not in the sandbox", display);
  fflush(stderr);
  exit(1);
}

#ifdef linux

// Interceptor for open
int open(const char *pathname, int flags, ...) {
  ensure_init();
  // open() takes a mode argument only when creating a file. On Linux mode_t is
  // unsigned int (rank == int), so it is NOT subject to default argument
  // promotion and must be read back as mode_t — reading it as int would be
  // undefined. (The macOS interceptors below read int instead, because there
  // mode_t is a 16-bit type that IS promoted to int when passed.)
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = va_arg(args, mode_t);
    va_end(args);
  }
  // If we are already inside the sandbox's own logic on this thread, this
  // open() is one of our internal calls (e.g. reading requisites.txt). Hand it
  // straight to the real function to avoid recursing back through the policy.
  if (in_sandbox)
    return orig_open(pathname, flags, mode);
  in_sandbox = 1;
  bool allowed = sandbox_check_path(pathname);
  in_sandbox = 0;
  if (allowed)
    return orig_open(pathname, flags, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for openat
int openat(int dirfd, const char *pathname, int flags, ...) {
  ensure_init();
  // See the note in open() above: on Linux mode_t is read back as mode_t.
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = va_arg(args, mode_t);
    va_end(args);
  }
  if (in_sandbox)
    return orig_openat(dirfd, pathname, flags, mode);
  in_sandbox = 1;
  bool allowed = sandbox_check_path(pathname);
  in_sandbox = 0;
  if (allowed)
    return orig_openat(dirfd, pathname, flags, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for fopen.
//
// Many programs (e.g. coreutils `sum`, `cksum`) open files with fopen()
// rather than open()/openat(). On Linux, glibc's fopen() routes through an
// internal symbol (__GI__IO_file_open → __libc_open64) that never touches the
// exported open/openat PLT entries, so those interceptors never fire. We must
// interpose fopen and fopen64 directly to catch this path.
FILE *fopen(const char *pathname, const char *mode) {
  ensure_init();
  if (in_sandbox)
    return orig_fopen(pathname, mode);
  in_sandbox = 1;
  bool allowed = sandbox_check_path(pathname);
  in_sandbox = 0;
  if (allowed)
    return orig_fopen(pathname, mode);
  errno = EACCES;
  return NULL;
}

// Interceptor for fopen64 (large-file alias; distinct PLT entry on Linux even
// though it maps to the same implementation on 64-bit systems).
FILE *fopen64(const char *pathname, const char *mode) {
  ensure_init();
  if (in_sandbox)
    return orig_fopen64(pathname, mode);
  in_sandbox = 1;
  bool allowed = sandbox_check_path(pathname);
  in_sandbox = 0;
  if (allowed)
    return orig_fopen64(pathname, mode);
  errno = EACCES;
  return NULL;
}

// Interceptor for readlinkat. Reading a symlink reveals an out-of-closure
// path, which some build tools rely on instead of open(); it is flagged so the
// dependency is visible. But a symlink read is "looking around", not consuming
// out-of-closure contents, so (via in_readlink) it is warned-but-permitted even
// under enforce, the same as a directory listing.
ssize_t readlinkat(int dirfd, const char *pathname, char *buf, size_t bufsiz) {
  ensure_init();
  if (in_sandbox)
    return orig_readlinkat(dirfd, pathname, buf, bufsiz);
  in_sandbox = 1;
  in_readlink = 1;
  bool allowed = sandbox_check_path(pathname);
  in_readlink = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_readlinkat(dirfd, pathname, buf, bufsiz);
  errno = EACCES;
  return -1;
}

// Interceptor for readlink (the non-at POSIX form). Same semantics as
// readlinkat: warned-but-permitted even under enforce.
ssize_t readlink(const char *pathname, char *buf, size_t bufsiz) {
  ensure_init();
  if (in_sandbox)
    return orig_readlink(pathname, buf, bufsiz);
  in_sandbox = 1;
  in_readlink = 1;
  bool allowed = sandbox_check_path(pathname);
  in_readlink = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_readlink(pathname, buf, bufsiz);
  errno = EACCES;
  return -1;
}

// Interceptor for __readlink_chk — the _FORTIFY_SOURCE=2 variant of readlink.
// Coreutils (ls, readlink, realpath) and most binaries compiled with
// -D_FORTIFY_SOURCE=2 bind to this name rather than plain readlink, so without
// this interceptor symlink reads in those tools slip past the sandbox entirely.
ssize_t __readlink_chk(const char *pathname, char *buf, size_t bufsiz,
                       size_t buflen) {
  ensure_init();
  if (in_sandbox)
    return orig_readlink_chk(pathname, buf, bufsiz, buflen);
  in_sandbox = 1;
  in_readlink = 1;
  bool allowed = sandbox_check_path(pathname);
  in_readlink = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_readlink_chk(pathname, buf, bufsiz, buflen);
  errno = EACCES;
  return -1;
}

// Interceptor for __readlinkat_chk — the _FORTIFY_SOURCE=2 variant of
// readlinkat. Same semantics as the plain readlinkat interceptor.
ssize_t __readlinkat_chk(int dirfd, const char *pathname, char *buf,
                         size_t bufsiz, size_t buflen) {
  ensure_init();
  if (in_sandbox)
    return orig_readlinkat_chk(dirfd, pathname, buf, bufsiz, buflen);
  in_sandbox = 1;
  in_readlink = 1;
  bool allowed = sandbox_check_path(pathname);
  in_readlink = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_readlinkat_chk(dirfd, pathname, buf, bufsiz, buflen);
  errno = EACCES;
  return -1;
}

#else

// Interceptor for open.
//
// On macOS we reach the real open() simply by calling open(): dyld
// interposition does not redirect calls made from within the library that
// defines the interposers, so a self-call lands on libc rather than recursing
// back into my_open().
int my_open(const char *pathname, int flags, ...) {
  ensure_init();
  debug("my_open('%s'), sandbox_level=%d", pathname, sandbox_level);
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }
  // Re-entrancy guard: internal opens performed by our own checks (e.g. the
  // libc fopen() of requisites.txt, which is NOT a self-call and therefore IS
  // interposed) would otherwise recurse back through here. Let them pass
  // straight through to the real open().
  if (in_sandbox)
    return open(pathname, flags, mode);
  in_sandbox = 1;
  bool allowed = sandbox_check_path(pathname);
  in_sandbox = 0;
  if (allowed)
    return open(pathname, flags, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for openat
int my_openat(int dirfd, const char *pathname, int flags, ...) {
  ensure_init();
  debug("my_openat('%s'), sandbox_level=%d", pathname, sandbox_level);
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }
  if (in_sandbox)
    return openat(dirfd, pathname, flags, mode);
  in_sandbox = 1;
  bool allowed = sandbox_check_path(pathname);
  in_sandbox = 0;
  if (allowed)
    return openat(dirfd, pathname, flags, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for fopen (macOS).
FILE *my_fopen(const char *pathname, const char *mode) {
  ensure_init();
  if (in_sandbox)
    return fopen(pathname, mode);
  in_sandbox = 1;
  bool allowed = sandbox_check_path(pathname);
  in_sandbox = 0;
  if (allowed)
    return fopen(pathname, mode);
  errno = EACCES;
  return NULL;
}

// Interceptor for readlinkat (macOS). Like the Linux one: a symlink read is
// "looking around", so it is warned-but-permitted even under enforce (via
// in_readlink), not blocked.
ssize_t my_readlinkat(int dirfd, const char *pathname, char *buf,
                      size_t bufsiz) {
  ensure_init();
  if (in_sandbox)
    return readlinkat(dirfd, pathname, buf, bufsiz);
  in_sandbox = 1;
  in_readlink = 1;
  bool allowed = sandbox_check_path(pathname);
  in_readlink = 0;
  in_sandbox = 0;
  if (allowed)
    return readlinkat(dirfd, pathname, buf, bufsiz);
  errno = EACCES;
  return -1;
}

// Thank you https://www.emergetools.com/blog/posts/DyldInterposing
#define DYLD_INTERPOSE(_replacement, _replacee)                                \
  __attribute__((used)) static struct {                                        \
    const void *replacement;                                                   \
    const void *replacee;                                                      \
  } _interpose_##_replacee __attribute__((section("__DATA,__interpose"))) = {  \
      (const void *)(unsigned long)&_replacement,                              \
      (const void *)(unsigned long)&_replacee};
DYLD_INTERPOSE(my_open, open)
DYLD_INTERPOSE(my_openat, openat)
DYLD_INTERPOSE(my_fopen, fopen)
DYLD_INTERPOSE(my_readlinkat, readlinkat)

// macOS exports a second fopen, fopen$DARWIN_EXTSN (the "extended standards"
// variant). Binaries built in Darwin C mode — including the Nix coreutils
// `sum`/`cksum` — bind to it rather than plain fopen, so interposing only
// `fopen` lets those file reads slip past the sandbox. Interpose the variant
// too, via an asm-aliased declaration of its real symbol name.
extern FILE *
fopen_darwin_extsn(const char *pathname,
                   const char *mode) __asm__("_fopen$DARWIN_EXTSN");
DYLD_INTERPOSE(my_fopen, fopen_darwin_extsn)

#endif
