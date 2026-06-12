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
 *
 * Threat model: this is friction plus audit for cooperative tools, not
 * containment. Interposition only mediates libc entry points (raw syscalls
 * and statically-linked binaries bypass it entirely), and every path check
 * is check-then-act: the policy decision resolves the name, then the real
 * call re-resolves it, so a symlink swapped in between defeats the check.
 * That race is accepted as inside the cooperative-tools threat model.
 */

#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <fnmatch.h>
#include <limits.h>
#include <netdb.h>
#include <netinet/in.h>
#include <poll.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

// Declare version bindings to work with minimum supported GLIBC versions.
#ifdef linux
#include "glibc-bindings.h"
#else
// _NSGetExecutablePath, used to find the process executable on macOS (no
// /proc).
#include <mach-o/dyld.h>
#endif

// For access to the in_closure() function.
#include "closure.h"

// Audit level derived from FLOX_VIRTUAL_SANDBOX. The numeric values are a
// total order from "do nothing" to "block everything out of policy"; new
// code compares against the named constants rather than bare integers so the
// intent is legible. The historical literals (0..3) are unchanged so the
// warn/enforce/pure behaviour stays byte-identical.
#define SANDBOX_LEVEL_UNINIT (-1) // not yet initialized
#define SANDBOX_LEVEL_OFF 0       // no checking
#define SANDBOX_LEVEL_WARN 1      // report out-of-closure access, permit it
#define SANDBOX_LEVEL_ENFORCE 2 // report out-of-closure access, refuse (EACCES)
#define SANDBOX_LEVEL_PURE 3    // enforce, but inside the Nix sandbox
// "prompt" is enforce plus an interactive escape hatch: out-of-policy access
// is referred to an external broker over the FLOX_SANDBOX_PROMPT_SOCKET Unix
// socket (the per-activation flox-activations executive, or the per-build
// flox prompt broker) for an allow/deny verdict. It is not a distinct level:
// the parser maps it to SANDBOX_LEVEL_ENFORCE and sets prompt_mode. Under an
// ACTIVATION (allow_foreign_exe set), a dead or absent broker denies
// gracefully (EACCES) with a distinct fail-closed receipt rather than
// aborting, so prompt degrades to enforce-with-better-errors; for a BUILD the
// no-broker case is plain enforce.

// Written exactly once (under init_once, via ensure_init) and only read
// afterwards, so it needs no further synchronization.
int sandbox_level = SANDBOX_LEVEL_UNINIT;

// The grants directory backs the write guard and the audit store, read once
// from the environment during init. Without a grants dir both are inert
// (builds never set it). A pointer into the environment block, never freed or
// mutated after init.
static const char *sandbox_grants_dir = NULL;
// Resolved realpath of sandbox_grants_dir, resolved lazily on first use (NOT in
// init — see the note in sandbox_init) so the write guard can do a
// boundary-aware prefix compare against realpaths. Guarded by
// grants_dir_resolve_once; after that runs, grants_dir_real_len == 0 means the
// grants dir was unset or unresolvable and the write guard is inert. The buffer
// is written exactly once under pthread_once and only read afterwards.
static char grants_dir_real[PATH_MAX];
static size_t grants_dir_real_len = 0;
static pthread_once_t grants_dir_resolve_once = PTHREAD_ONCE_INIT;

// When true, the executable-identity check is skipped entirely. A build runs
// the toolchain from inside the environment closure, so a process executable
// from outside it signals the wrong toolchain is active — a reproducibility
// defect worth reporting (warn) or aborting on (enforce/pure). An activation
// is the opposite: it deliberately runs the user's shell and host tools (the
// coding agent, git, python) from outside the closure, and mediates only file
// and network ACCESS, not executable identity. The activation injects
// FLOX_SANDBOX_ALLOW_FOREIGN_EXE so the foreign-executable check does not fire
// on the inner shell. Read once during init; builds never set it, so build
// behaviour is unchanged.
static bool allow_foreign_exe = false;

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

// Per-thread flag marking the current access as a write (or read-write /
// append) rather than a pure read. Each interceptor derives this from the open
// flags or fopen mode and sets it around its sandbox_check_path() call. Today
// it feeds the audit op column, the receipt's read/write wording, and the
// grants-dir write guard. Enforcement otherwise stays access-agnostic, so
// the verdict never depends on this flag.
static __thread int in_write_access = 0;

// Per-thread flag marking the current open()/openat() as an O_DIRECTORY probe.
// An open with O_DIRECTORY cannot read file contents — the kernel returns
// ENOTDIR for any non-directory path, so no out-of-closure data can escape.
// Like readlinkat, it is "looking around" rather than consuming file contents,
// so it is warned-but-permitted even under enforce. The open/openat
// interceptors set this when O_DIRECTORY is in the flags; sandbox_check_path()
// consults it.
static __thread int in_dir_probe = 0;

// Per-thread flag marking the current check as a directory ENUMERATION
// (opendir()/fdopendir()). Unlike an O_DIRECTORY probe — which only yields an
// fd — an enumeration hands the caller every entry name in the directory, so
// under an activation it is mediated as a READ of the directory path (per the
// unified severity model) rather than warn-permitted. The opendir/fdopendir
// interceptors set this around their sandbox_check_path() call;
// sandbox_check_path() consults it.
static __thread int in_dir_read = 0;

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
// Additional write entry points. A write must not slip past the sandbox by
// binding a libc symbol the open()/fopen() interceptors never saw: creat() is
// open(O_WRONLY|O_CREAT|O_TRUNC) under another name; open64()/openat64()/
// creat64() are the _FILE_OFFSET_BITS=64 large-file variants that 64-bit builds
// bind instead of the plain names; truncate() is a path-addressed destructive
// write; freopen()/freopen64() open (and truncate) a path behind a FILE*.
static int (*orig_creat)(const char *pathname, mode_t mode) = NULL;
static int (*orig_open64)(const char *pathname, int flags, ...) = NULL;
static int (*orig_openat64)(int dirfd, const char *pathname, int flags,
                            ...) = NULL;
static int (*orig_creat64)(const char *pathname, mode_t mode) = NULL;
static int (*orig_truncate)(const char *path, off_t length) = NULL;
static FILE *(*orig_freopen)(const char *pathname, const char *mode,
                             FILE *stream) = NULL;
static FILE *(*orig_freopen64)(const char *pathname, const char *mode,
                               FILE *stream) = NULL;
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
// Directory enumeration. ls and shell globs list a directory with
// opendir()/readdir(); glibc's opendir() opens the directory through an
// internal __open*_nocancel symbol that never touches the exported
// open()/openat() PLT entries, so the open interceptors above cannot see an
// enumeration. fdopendir() covers the openat()+fdopendir() traversal style
// (find, fts).
static DIR *(*orig_opendir)(const char *name) = NULL;
static DIR *(*orig_fdopendir)(int fd) = NULL;
// Network egress. connect() is the TCP egress choke point; getaddrinfo() is
// observed only to attach a best-effort hostname to the resolved IPs.
static int (*orig_connect)(int sockfd, const struct sockaddr *addr,
                           socklen_t addrlen) = NULL;
static int (*orig_getaddrinfo)(const char *node, const char *service,
                               const struct addrinfo *hints,
                               struct addrinfo **res) = NULL;
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
// Program name for message attribution. Every SANDBOX line is tagged
// [exe:pid] so a report can be traced to the process that triggered it —
// a bare PID is useless once the process exits (e.g. flox's own short-lived
// metrics phone-home was mistaken for blocked curl requests). glibc exposes
// the invocation basename via program_invocation_short_name (declared in
// <errno.h> under _GNU_SOURCE); macOS provides getprogname() in <stdlib.h>.
#ifdef linux
#define SANDBOX_PROGNAME program_invocation_short_name
#else
#define SANDBOX_PROGNAME getprogname()
#endif
#define debug(format, ...)                                                     \
  do {                                                                         \
    if (debug_sandbox)                                                         \
      fprintf(stderr, "SANDBOX DEBUG[%s:%d]: " format "\n", SANDBOX_PROGNAME,  \
              getpid(), __VA_ARGS__);                                          \
  } while (0)
#define warn(format, ...)                                                      \
  fprintf(stderr, "SANDBOX WARNING[%s:%d]: " format "\n", SANDBOX_PROGNAME,    \
          getpid(), ##__VA_ARGS__)
#define warn_once(format, ...)                                                 \
  do {                                                                         \
    if (debug_sandbox)                                                         \
      warn(format, ##__VA_ARGS__);                                             \
    else if (atomic_fetch_add_explicit(&warn_count, 1,                         \
                                       memory_order_relaxed) == 0)             \
      warn(format " (further warnings suppressed)", ##__VA_ARGS__);            \
  } while (0)
#define _error(format, ...)                                                    \
  fprintf(stderr, "SANDBOX ERROR[%s:%d]: " format "\n", SANDBOX_PROGNAME,      \
          getpid(), ##__VA_ARGS__)
#define hint(format, ...)                                                      \
  fprintf(stderr, "SANDBOX HINT[%s:%d]: " format "\n", SANDBOX_PROGNAME,       \
          getpid(), ##__VA_ARGS__)
// A denial receipt under prompt: the access was refused (and, under an
// activation, queued for approval outside the session). Distinct prefix from
// WARNING/ERROR because it is neither — the operation failed cleanly and can
// be redeemed by retry once
// approved.
#define denied(format, ...)                                                    \
  fprintf(stderr, "SANDBOX DENIED[%s:%d]: " format "\n", SANDBOX_PROGNAME,     \
          getpid(), ##__VA_ARGS__)

// Resolved realpath of the user's $HOME and its length, captured once during
// initialization. Used to recognize "$HOME/.<dotfile>" accesses (see
// is_home_dotfile below). home_real_len == 0 means $HOME was unset or could
// not be resolved, in which case no path is treated as a home dotfile.
static char home_real[PATH_MAX];
static size_t home_real_len = 0;

// Interactive prompt broker (Phase 2). In "prompt" mode an out-of-closure
// access — instead of being blocked outright — is referred to a broker (the
// flox process driving the build) at FLOX_SANDBOX_PROMPT_SOCKET, which prompts
// the user and replies allow / deny / allow-glob.
//
// prompt_mode is true only when FLOX_VIRTUAL_SANDBOX == "prompt"; it gates the
// broker consultation, so an enforce-mode build that happens to share the
// process-wide socket env (a mixed `flox build`) never prompts. prompt_enabled
// is true when a socket path is set; it gates the (otherwise lock-free)
// glob-list reads, since the broker path can append to the list at runtime (see
// allow_globs_add). With prompt_mode but no socket — a non-interactive build —
// prompt_broker errors and the access falls through to enforce.
static char prompt_socket_path[PATH_MAX];
static int prompt_enabled = 0;
static int prompt_mode = 0;

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

  // Note the interactive prompt broker socket, if any.
  const char *prompt_socket = getenv("FLOX_SANDBOX_PROMPT_SOCKET");
  if (prompt_socket != NULL && prompt_socket[0] != '\0' &&
      strlen(prompt_socket) < sizeof(prompt_socket_path)) {
    strncpy(prompt_socket_path, prompt_socket, sizeof(prompt_socket_path) - 1);
    prompt_socket_path[sizeof(prompt_socket_path) - 1] = '\0';
    prompt_enabled = 1;
  }

  // Derive audit level from FLOX_VIRTUAL_SANDBOX environment variable.
  const char *flox_virtual_sandbox_value = getenv("FLOX_VIRTUAL_SANDBOX");
  if (flox_virtual_sandbox_value == NULL ||
      (strcmp(flox_virtual_sandbox_value, "off") == 0)) {
    sandbox_level = SANDBOX_LEVEL_OFF;
  } else if (strcmp(flox_virtual_sandbox_value, "warn") == 0) {
    sandbox_level = SANDBOX_LEVEL_WARN;
  } else if (strcmp(flox_virtual_sandbox_value, "enforce") == 0) {
    sandbox_level = SANDBOX_LEVEL_ENFORCE;
  } else if (strcmp(flox_virtual_sandbox_value, "prompt") == 0) {
    // Prompt mode is enforce with an interactive escape hatch: out-of-closure
    // accesses are referred to the prompt broker (when
    // FLOX_SANDBOX_PROMPT_SOCKET is set; see prompt_broker) and otherwise
    // blocked. With no broker — e.g. a non-interactive build — it is therefore
    // just enforce.
    sandbox_level = SANDBOX_LEVEL_ENFORCE;
    prompt_mode = 1;
  } else if (strcmp(flox_virtual_sandbox_value, "pure") == 0) {
    // Pure mode is just like enforce, but invoked within the Nix sandbox.
    sandbox_level = SANDBOX_LEVEL_PURE;
  } else {
    warn_once("FLOX_VIRTUAL_SANDBOX must be (off|warn|enforce|prompt|pure) "
              "... ignoring");
    sandbox_level = SANDBOX_LEVEL_OFF;
  }
  debug("sandbox_level=%d prompt_mode=%d", sandbox_level, prompt_mode);

  // Capture the grants directory for the write guard and the audit store. It
  // may be absent: without a grants dir both are inert.
  //
  // The grants dir is captured here but NOT realpath()'d here. Resolving it in
  // init is unsafe on macOS: when DYLD_INSERT_LIBRARIES is active, a realpath()
  // that traverses a symlink during this very early constructor-time init (e.g.
  // a grants dir under /tmp or /private/var, both symlinked) makes dyld lazily
  // load delayed system dylibs (Directory Services / LDAP / libsasl) and hard-
  // kills the process. The resolution is deferred to first use in the write
  // guard (grants_dir_real_resolved), by which point the process is fully up
  // and a symlink-traversing realpath() is safe.
  sandbox_grants_dir = getenv("FLOX_SANDBOX_GRANTS_DIR");

  // Activation injects FLOX_SANDBOX_ALLOW_FOREIGN_EXE so the foreign-executable
  // check (a build-reproducibility heuristic) does not abort on the inner
  // shell. Any non-empty value enables it; builds never set it.
  const char *allow_foreign_exe_value =
      getenv("FLOX_SANDBOX_ALLOW_FOREIGN_EXE");
  allow_foreign_exe =
      allow_foreign_exe_value != NULL && allow_foreign_exe_value[0] != '\0';

#ifdef linux
  // Declare new functions to be intercepted here, then add stub
  // functions below.
  orig_open = dlsym(RTLD_NEXT, "open");
  orig_openat = dlsym(RTLD_NEXT, "openat");
  orig_fopen = dlsym(RTLD_NEXT, "fopen");
  orig_fopen64 = dlsym(RTLD_NEXT, "fopen64");
  orig_creat = dlsym(RTLD_NEXT, "creat");
  orig_open64 = dlsym(RTLD_NEXT, "open64");
  orig_openat64 = dlsym(RTLD_NEXT, "openat64");
  orig_creat64 = dlsym(RTLD_NEXT, "creat64");
  orig_truncate = dlsym(RTLD_NEXT, "truncate");
  orig_freopen = dlsym(RTLD_NEXT, "freopen");
  orig_freopen64 = dlsym(RTLD_NEXT, "freopen64");
  orig_readlinkat = dlsym(RTLD_NEXT, "readlinkat");
  orig_readlink = dlsym(RTLD_NEXT, "readlink");
  orig_readlink_chk = dlsym(RTLD_NEXT, "__readlink_chk");
  orig_readlinkat_chk = dlsym(RTLD_NEXT, "__readlinkat_chk");
  orig_opendir = dlsym(RTLD_NEXT, "opendir");
  orig_fdopendir = dlsym(RTLD_NEXT, "fdopendir");
  orig_connect = dlsym(RTLD_NEXT, "connect");
  orig_getaddrinfo = dlsym(RTLD_NEXT, "getaddrinfo");
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

// One-time initializer for allow_dirs[], invoked via pthread_once() from
// check_allowed_basenames(). Running exactly once is what lets readers scan
// the list without locking: once this returns, allow_dirs[] and
// allow_dirs_count are never written again, so concurrent scans see a stable,
// immutable array.
static void allow_dirs_init(void) {
  // Copy FLOX_SANDBOX_ALLOW_DIRS into a writable buffer and tokenize it on
  // spaces, recording a pointer to each entry.
  const char *allow_dirs_env = getenv("FLOX_SANDBOX_ALLOW_DIRS");
  if (allow_dirs_env != NULL) {
    // Warn and truncate if the value would not fit in allow_dirs_buf. Measure
    // with strlen, not sizeof: sizeof on a pointer is 8 bytes, not the string
    // length.
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
// Entries point either into allow_globs_buf (manifest entries, parsed once) or
// at strdup()'d strings appended at runtime by allow_globs_add() when the
// prompt broker accepts a pattern; either way they are never freed or mutated,
// so `const char *`.
static const char *allow_globs[FLOX_SANDBOX_ALLOW_MAXENTRIES];
static int allow_globs_count = 0;
// Guards allow_globs[]/allow_globs_count against concurrent runtime appends
// (allow_globs_add) racing reads (check_allowed_globs). Only engaged when the
// prompt broker is active; without it the list is built once and read-only, so
// the default build path takes no lock. PTHREAD_MUTEX_INITIALIZER is correct on
// both platforms (unlike the removed zero-initialized mutex).
static pthread_mutex_t allow_globs_lock = PTHREAD_MUTEX_INITIALIZER;

// Append a pattern to the allow-globs list at runtime (used when the prompt
// broker returns allow/allow-glob). Thread-safe. The pattern is strdup()'d and
// intentionally never freed: it must live for the rest of the process.
static void allow_globs_add(const char *pattern) {
  pthread_mutex_lock(&allow_globs_lock);
  if (allow_globs_count < FLOX_SANDBOX_ALLOW_MAXENTRIES) {
    char *copy = strdup(pattern);
    if (copy != NULL)
      allow_globs[allow_globs_count++] = copy;
  }
  pthread_mutex_unlock(&allow_globs_lock);
}

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
  // The list is immutable after init unless the prompt broker is active and may
  // append to it, so only then do we take the lock around the scan.
  if (prompt_enabled)
    pthread_mutex_lock(&allow_globs_lock);
  bool matched = false;
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
      matched = true;
      break;
    }
  }
  if (prompt_enabled)
    pthread_mutex_unlock(&allow_globs_lock);
  return matched;
}

// Decision values returned by prompt_broker().
#define PROMPT_ERROR                                                           \
  (-1)                 // no broker, or the exchange failed: use default policy
#define PROMPT_DENY 0  // user/broker denied this access
#define PROMPT_ALLOW 1 // user/broker allowed this access

// Refer an out-of-closure access to the interactive prompt broker over the
// AF_UNIX socket named by FLOX_SANDBOX_PROMPT_SOCKET. The wire protocol is one
// request and one reply per connection, newline-terminated text:
//
//   -> "<realpath>\n"
//   <- "allow\n"                 allow this access (remembered for the exact
//   path)
//   <- "allow-glob <pattern>\n"  allow, and remember <pattern> for future
//   matches
//   <- "deny\n"                  deny this access
//   <- "deny <req>\n"            deny, queued for out-of-band review as
//                                request <req> (sent by the activation broker;
//                                the build broker sends the bare form)
//
// One connection per query keeps the client simple and lets the broker
// serialize prompts (it accepts and handles connections one at a time). Socket
// calls are not themselves interposed, so there is no re-entrancy here.
// Returns a PROMPT_* value; PROMPT_ERROR means fall back to the normal
// warn/enforce policy (the activation tail turns that into a graceful
// fail-closed deny instead). On PROMPT_DENY, *req_out carries the queued
// request id (0 when the broker sent the bare form).
//
// An interactive build broker legitimately blocks for minutes while the user
// decides at /dev/tty, so the build path waits indefinitely. The activation
// broker answers instantly from its grant table, so under an activation
// (allow_foreign_exe set) a 2s poll() bounds the wait — the only thing it can
// catch is a wedged executive, which must not hang the user's shell.
static int prompt_broker(const char *real_path, unsigned long *req_out) {
  *req_out = 0;
  if (!prompt_enabled)
    return PROMPT_ERROR;

  struct sockaddr_un addr;
  memset(&addr, 0, sizeof(addr));
  addr.sun_family = AF_UNIX;
  // The socket path must fit in sun_path. If it does not, treat the broker as
  // unreachable rather than connecting to a truncated path.
  if (strlen(prompt_socket_path) >= sizeof(addr.sun_path))
    return PROMPT_ERROR;
  strncpy(addr.sun_path, prompt_socket_path, sizeof(addr.sun_path) - 1);

  // The fd must be close-on-exec so a user-spawned child of the requesting
  // process never inherits the broker connection. Linux exposes SOCK_CLOEXEC
  // as a socket() flag; macOS does not, so set it with fcntl() there.
#ifdef SOCK_CLOEXEC
  int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
#else
  int fd = socket(AF_UNIX, SOCK_STREAM, 0);
  if (fd >= 0)
    fcntl(fd, F_SETFD, FD_CLOEXEC);
#endif
  if (fd < 0)
    return PROMPT_ERROR;

  if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
    close(fd);
    return PROMPT_ERROR;
  }

  // Send the realpath being queried.
  char request[PATH_MAX + 2];
  int request_len = snprintf(request, sizeof(request), "%s\n", real_path);
  if (request_len < 0 || (size_t)request_len >= sizeof(request) ||
      write(fd, request, (size_t)request_len) != request_len) {
    close(fd);
    return PROMPT_ERROR;
  }

  // Under an activation, bound the wait (see above). A timeout or poll error
  // is PROMPT_ERROR, which the activation tail fails closed.
  if (allow_foreign_exe) {
    struct pollfd pfd = {.fd = fd, .events = POLLIN};
    int pr = poll(&pfd, 1, 2000);
    if (pr <= 0) {
      close(fd); // timeout (pr==0) or error (pr<0)
      return PROMPT_ERROR;
    }
  }

  // Read the single reply line. The broker writes one short line and we read
  // once; that is sufficient for the small fixed vocabulary above.
  char reply[PATH_MAX + 32];
  ssize_t reply_len = read(fd, reply, sizeof(reply) - 1);
  close(fd);
  if (reply_len <= 0)
    return PROMPT_ERROR;
  reply[reply_len] = '\0';
  reply[strcspn(reply, "\n")] = '\0';

  if (strncmp(reply, "allow-glob ", 11) == 0) {
    allow_globs_add(reply + 11);
    debug("prompt broker allowed '%s' via glob '%s'", real_path, reply + 11);
    return PROMPT_ALLOW;
  }
  if (strcmp(reply, "allow") == 0) {
    // Remember the exact path so the same file is not queried again.
    allow_globs_add(real_path);
    debug("prompt broker allowed '%s'", real_path);
    return PROMPT_ALLOW;
  }
  if (strcmp(reply, "deny") == 0) {
    debug("prompt broker denied '%s'", real_path);
    return PROMPT_DENY;
  }
  if (strncmp(reply, "deny ", 5) == 0) {
    // Queued for out-of-band review; the req id shapes the deny receipt. A
    // malformed id parses as 0, which just yields the less specific receipt.
    *req_out = strtoul(reply + 5, NULL, 10);
    debug("prompt broker denied '%s' (queued as req %lu)", real_path, *req_out);
    return PROMPT_DENY;
  }
  debug("prompt broker gave unrecognized reply '%s' for '%s'", reply,
        real_path);
  return PROMPT_ERROR;
}

// Sensitive-path glob set. Under an ACTIVATION (allow_foreign_exe set), these
// patterns are denied even under enforce — before the $HOME-dotfile carve-out
// that would otherwise wave them through. They name the credential and secret
// files a coding agent must not read (~/.ssh, ~/.aws, ~/.netrc, ~/.env, ...).
//
// This is deliberately gated on the activation: a build never sets
// allow_foreign_exe, so the sensitive set is never consulted during a build
// and build-sandbox behaviour is byte-identical. The defaults can be replaced
// wholesale with FLOX_SANDBOX_SENSITIVE (space-separated, same `~/`-expanded
// glob format as FLOX_SANDBOX_ALLOW); when that env var is set, the compiled-in
// list is not used.
//
// Matched like check_allowed_globs: fnmatch() with flag 0 so `**` crosses
// directory separators, and a leading `~/` expanded against $HOME. Parsed once
// under sensitive_once; read-only afterwards.
#define FLOX_SANDBOX_SENSITIVE_MAXENTRIES 256
#define FLOX_SANDBOX_SENSITIVE_MAXLEN (16 * 1024)

// Compiled-in defaults, used when FLOX_SANDBOX_SENSITIVE is unset. `**/.env`
// and `**/.env.*` use no path anchor so a project-local dotenv anywhere in the
// tree is covered; the rest are anchored under $HOME via `~/`.
static const char *const SENSITIVE_DEFAULTS[] = {
    "~/.ssh/**", "~/.aws/**",       "~/.gnupg/**", "~/.kube/**",
    "~/.netrc",  "~/.config/gh/**", "**/.env",     "**/.env.*",
};
#define SENSITIVE_DEFAULTS_COUNT                                               \
  (sizeof(SENSITIVE_DEFAULTS) / sizeof(SENSITIVE_DEFAULTS[0]))

static pthread_once_t sensitive_once = PTHREAD_ONCE_INIT;
static char sensitive_buf[FLOX_SANDBOX_SENSITIVE_MAXLEN];
// Read-only after init: either tokens inside sensitive_buf (env override) or
// the string literals in SENSITIVE_DEFAULTS.
static const char *sensitive_globs[FLOX_SANDBOX_SENSITIVE_MAXENTRIES];
static int sensitive_count = 0;

static void sensitive_init(void) {
  const char *env = getenv("FLOX_SANDBOX_SENSITIVE");
  if (env == NULL) {
    // No override: use the compiled-in defaults verbatim.
    for (size_t i = 0; i < SENSITIVE_DEFAULTS_COUNT; i++)
      sensitive_globs[sensitive_count++] = SENSITIVE_DEFAULTS[i];
    return;
  }
  // Override present: tokenize it the same way the allow globs are tokenized.
  if (strlen(env) >= sizeof(sensitive_buf)) {
    _error("FLOX_SANDBOX_SENSITIVE is too long, truncating to %zu characters",
           sizeof(sensitive_buf) - 1);
    fflush(stderr);
  }
  strncpy(sensitive_buf, env, sizeof(sensitive_buf) - 1);
  sensitive_buf[sizeof(sensitive_buf) - 1] = '\0';

  char *saveptr = NULL;
  char *pattern = strtok_r(sensitive_buf, " ", &saveptr);
  while (pattern != NULL) {
    if (sensitive_count >= FLOX_SANDBOX_SENSITIVE_MAXENTRIES) {
      _error("FLOX_SANDBOX_SENSITIVE has too many entries, using the first %d",
             FLOX_SANDBOX_SENSITIVE_MAXENTRIES);
      fflush(stderr);
      break;
    }
    debug("FLOX_SANDBOX_SENSITIVE pattern[%d] = %s", sensitive_count, pattern);
    sensitive_globs[sensitive_count++] = pattern;
    pattern = strtok_r(NULL, " ", &saveptr);
  }
}

// Returns true if `real_path` matches any sensitive glob. Mirrors
// check_allowed_globs: a leading `~/` is expanded against $HOME, and fnmatch()
// runs with flag 0 so `**` spans directory separators.
static bool path_is_sensitive(const char *real_path) {
  pthread_once(&sensitive_once, sensitive_init);
  for (int i = 0; i < sensitive_count; i++) {
    const char *pattern = sensitive_globs[i];
    char expanded[PATH_MAX];
    if (pattern[0] == '~' && pattern[1] == '/' && home_real_len > 0 &&
        (size_t)snprintf(expanded, sizeof(expanded), "%s%s", home_real,
                         pattern + 1) < sizeof(expanded)) {
      pattern = expanded;
    }
    if (fnmatch(pattern, real_path, 0) == 0) {
      debug("%s matches sensitive pattern '%s'", real_path, sensitive_globs[i]);
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

// Returns true if `path` is `prefix` itself or lies beneath it, matching on a
// path-component boundary. This is the same boundary discipline the allow-dirs
// scan applies inline (a textual prefix alone would let "/a/bc" match "/a/b"),
// factored out so the grants-dir write guard can reuse it. Both arguments must
// already be realpaths; `prefix_len` is strlen(prefix), passed in because the
// guard captures it once at init.
static bool path_under(const char *path, const char *prefix,
                       size_t prefix_len) {
  if (prefix_len == 0)
    return false;
  return strncmp(path, prefix, prefix_len) == 0 &&
         (path[prefix_len] == '/' || path[prefix_len] == '\0');
}

// pthread_once body: resolve the grants dir to a realpath into grants_dir_real.
// Deferred out of init because a symlink-traversing realpath() at init time
// hard-kills the process on macOS under DYLD_INSERT_LIBRARIES (see
// sandbox_init).
static void resolve_grants_dir(void) {
  if (sandbox_grants_dir != NULL &&
      realpath(sandbox_grants_dir, grants_dir_real) != NULL)
    grants_dir_real_len = strlen(grants_dir_real);
}

// Return the resolved grants-dir realpath length, resolving it on first call.
// grants_dir_real is valid only after this returns; a length of 0 means the
// grants dir was unset or unresolvable (the write guard is then inert). Safe
// under in_sandbox==1: realpath()'s internal opens pass through, and the
// pthread_once is a distinct one-shot from the main init_once.
static size_t grants_dir_real_length(void) {
  pthread_once(&grants_dir_resolve_once, resolve_grants_dir);
  return grants_dir_real_len;
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

// Report once per process if the process executable itself is outside the
// closure. Called from sandbox_check_path() the first time any out-of-closure
// path is detected, so the user sees the root cause ("the wrong Node.js is
// active") before any downstream path message. This is ALWAYS a warning, never
// fatal on its own: it is context, not the access itself. Whether to abort is
// left to the per-path logic below, so the advisory accesses (readlinkat,
// O_DIRECTORY probes, directory listings, $HOME dotfiles) stay warned-but-
// permitted even when the executable is out of closure; an actual out-of-
// closure content read is still fatal under enforce, now with this root-cause
// line printed first. Safe to call while in_sandbox==1: in_closure() re-uses
// the already-initialized closure table (pthread_once is a no-op) and
// realpath() only touches libc internals that bypass our interceptors.
static void maybe_report_process_outside_closure(void) {
  // Activation deliberately runs host tools (the inner shell, the coding agent,
  // git, python) from outside the environment closure, so the executable-
  // identity check that guards build reproducibility does not apply. Skip it
  // entirely — neither warn nor abort — when the activation opted in. File and
  // network access mediation is unaffected; only the exe check changes.
  if (allow_foreign_exe)
    return;

  // Print the root-cause line at most once. Unlike home_dotfile_hint() — which
  // can safely claim its guard up front because nothing between the guard and
  // its message can fail — this function has early returns *between* the guard
  // and the warning: the exe path may not resolve. Claiming the guard first
  // would therefore suppress the report for the life of the process if that
  // first resolution failed. Instead: cheap-exit if a verdict was already
  // delivered, resolve first, and claim the guard only once we hold a path
  // (i.e. a final verdict to deliver).
  static atomic_int done = 0;
  if (atomic_load_explicit(&done, memory_order_relaxed) != 0)
    return;

  // Resolve the running executable's real path. Linux exposes it as the
  // /proc/self/exe symlink; macOS has no /proc, so ask dyld for the image path
  // and canonicalize that. A resolution failure may be transient, so return
  // WITHOUT claiming the guard, leaving a later out-of-closure access to retry.
  char argv0_real[PATH_MAX];
#ifdef linux
  if (realpath("/proc/self/exe", argv0_real) == NULL)
    return;
#else
  char exe_path[PATH_MAX];
  uint32_t exe_size = sizeof(exe_path);
  if (_NSGetExecutablePath(exe_path, &exe_size) != 0)
    return; // path did not fit (exe_size is set to the required size)
  if (realpath(exe_path, argv0_real) == NULL)
    return;
#endif

  // We hold a stable exe path (it does not change over a process's life), so
  // the verdict below is final. Claim the one-shot guard now; if another thread
  // won the race it is already delivering the single report.
  if (atomic_fetch_add_explicit(&done, 1, memory_order_relaxed) != 0)
    return;

  // Skip the same permitted executables that sandbox_check_argv0() exempts.
  if (strcmp(argv0_real, "/usr/bin/env") == 0 ||
      strcmp(argv0_real, "/bin/sh") == 0 ||
      strcmp(argv0_real, "/usr/bin/dash") == 0)
    return;
  // Mirror the same allow-list checks that sandbox_check_path() applies to
  // regular paths: allowed dirs, user globs, then the closure itself.
  if (check_allowed_basenames(argv0_real) || check_allowed_globs(argv0_real) ||
      in_closure(argv0_real))
    return;
  warn("process executable %s is outside the environment closure; "
       "subsequent file accesses by this process may not be reproducible",
       argv0_real);
}

// True if `real_path` is `/nix/store` itself or lies beneath it, on a
// path-component boundary. The Nix store is immutable, content-addressed,
// world-readable public packages, so for an activation a store path is always
// in policy. This mirrors the activation seed (which adds /nix/store to the
// allow-dirs); it is duplicated in the engine because the parent-dir create
// check below must recognize a store parent independently of the seed.
static bool path_in_nix_store(const char *real_path) {
  static const char store[] = "/nix/store";
  static const size_t store_len = sizeof(store) - 1;
  return strncmp(real_path, store, store_len) == 0 &&
         (real_path[store_len] == '/' || real_path[store_len] == '\0');
}

// True if `real_path` (a resolved realpath) is permitted by the standard allow
// set: an allowed basename/dir prefix, a user-declared sandbox-allow glob, the
// environment closure, or the Nix store. This is the same battery of checks
// sandbox_check_path applies to a regular access, factored out so the
// activation create-guard can ask the same question of a create's parent
// directory.
static bool path_in_policy(const char *real_path) {
  return check_allowed_basenames(real_path) || check_allowed_globs(real_path) ||
         in_closure(real_path) || path_in_nix_store(real_path);
}

// For a write that creates a nonexistent path, decide whether the create is in
// policy by resolving the path's nearest EXISTING ancestor directory and
// running it through path_in_policy(). A create under an in-policy directory
// (the project tree, the closure, the store) is allowed; a create anywhere
// else is out of policy.
//
// `pathname` is the path as opened (the target does not exist, so it has no
// realpath of its own). We copy it into a stack buffer and walk up component by
// component — trimming the trailing path element each time realpath() fails —
// until an ancestor resolves. This handles a create inside a not-yet-existing
// subtree (e.g. git writing `.git/objects/ce/tmp_obj_*` before the `ce` fanout
// dir exists, or any tool doing a deep create): the create is judged by the
// deepest directory that actually exists, which is the directory the new
// subtree will be rooted under. Creating a new subtree under an allowed
// directory is itself an allowed in-project write.
//
// All work is on a stack-local copy, never the caller's string and never a
// shared static, so this is safe under in_sandbox==1 and across threads. A
// relative pathname is resolved against the process cwd by realpath(), exactly
// as the open() it guards would be. Returns false only if NO ancestor resolves
// (which cannot normally happen — "/" always resolves) or the path is too long.
static bool create_parent_in_policy(const char *pathname) {
  char copy[PATH_MAX];
  if (strlen(pathname) >= sizeof(copy))
    return false; // too long to reason about; treat as out of policy

  // Start from the path's parent directory, then walk up.
  strncpy(copy, pathname, sizeof(copy) - 1);
  copy[sizeof(copy) - 1] = '\0';
  char *last_slash = strrchr(copy, '/');
  if (last_slash == NULL) {
    // Relative leaf with no '/': the parent is the current directory.
    char cwd_real[PATH_MAX];
    if (realpath(".", cwd_real) == NULL)
      return false;
    return path_in_policy(cwd_real);
  }
  if (last_slash == copy) {
    // Path is "/name": the parent is the root directory.
    char root_real[PATH_MAX];
    if (realpath("/", root_real) == NULL)
      return false;
    return path_in_policy(root_real);
  }
  *last_slash = '\0';

  // Walk up: try to resolve the current candidate; if it does not exist, trim
  // its last component and retry. Each iteration shortens `copy`, so the loop
  // terminates (in the worst case at the root, handled above).
  char ancestor_real[PATH_MAX];
  for (;;) {
    if (realpath(copy, ancestor_real) != NULL)
      return path_in_policy(ancestor_real);
    char *slash = strrchr(copy, '/');
    if (slash == NULL) {
      // No more separators: the remaining candidate is a relative element
      // under the cwd, so judge the cwd.
      char cwd_real[PATH_MAX];
      if (realpath(".", cwd_real) == NULL)
        return false;
      return path_in_policy(cwd_real);
    }
    if (slash == copy) {
      // Trimmed down to "/something" that did not resolve: judge the root.
      char root_real[PATH_MAX];
      if (realpath("/", root_real) == NULL)
        return false;
      return path_in_policy(root_real);
    }
    *slash = '\0';
  }
}

// ===========================================================================
// prompt flow: deny cache + receipts around the prompt broker client.
//
// In prompt mode, an out-of-policy access asks the broker named by
// FLOX_SANDBOX_PROMPT_SOCKET for a verdict (see prompt_broker above). Two
// pieces make that cheap and fail-safe for a long-lived activation session:
//
//   - allow verdicts are cached by the client itself (allow_globs_add), so
//     one allow-glob answer silences a whole subtree with no further RPCs;
//   - deny verdicts are cached here per exact path with a short TTL, so a
//     denied path retried in a tight loop does not storm the broker, while an
//     approval granted out-of-band ('flox sandbox allow') is redeemable by a
//     quick retry;
//   - a fail-closed path for activations: any socket trouble denies with a
//     distinct receipt, so a dead broker degrades prompt to
//     enforce-with-better-errors rather than silently unsandboxing.
//
// Only the filesystem flow is wired to the broker; network prompt keeps the
// interim enforce semantics handled in sandbox_check_connect().
// ===========================================================================

// Deny cache (modeled on warned_paths): exact realpaths, matched literally,
// with a short absolute expiry (wall-clock seconds) so a retry after the TTL
// re-asks the broker and can pick up a freshly added grant.
// Mutex-guarded like warned_paths; the same pthread_atfork exposure applies
// (a child forked while the lock is held could deadlock on the cache), which
// is accepted here exactly as it is for warned_paths: the lock is only taken
// on the cold out-of-policy path, never the hot in-closure path.
#define DENY_CACHE_MAX 4096
// Deny cache lifetime in seconds. Short on purpose: a deny should not pin a
// path closed for long, so an approval granted out-of-band is redeemable by a
// quick retry.
#define PROMPT_DENY_TTL_SECONDS 2
struct deny_entry {
  char path[PATH_MAX]; // exact realpath
  time_t expiry;       // absolute wall-clock seconds
};
static struct deny_entry deny_cache[DENY_CACHE_MAX];
static int deny_cache_count = 0;
static pthread_mutex_t deny_cache_lock = PTHREAD_MUTEX_INITIALIZER;

// Wall-clock seconds, for the short deny TTL. time() (not clock_gettime) is
// used deliberately: clock_gettime lived in librt rather than libc at the
// x86_64 baseline (GLIBC_2.2.5) and would either raise this library's glibc
// floor or pull in an extra link dependency, whereas time() has been in libc
// since the baseline for every supported arch. A 2-second TTL is short enough
// that a wall-clock jump is not a meaningful concern for a prototype. time()
// is not intercepted, so this is safe under in_sandbox==1.
static time_t deny_now_seconds(void) { return time(NULL); }

// Return true if `real_path` has a still-fresh cached deny. Expired entries
// are treated as misses so the caller re-asks the broker.
static bool deny_cache_hit(const char *real_path) {
  time_t now = deny_now_seconds();
  bool found = false;
  pthread_mutex_lock(&deny_cache_lock);
  for (int i = 0; i < deny_cache_count; i++) {
    struct deny_entry *v = &deny_cache[i];
    // An entry past its TTL is stale; skip it so the path re-asks.
    if (now >= v->expiry)
      continue;
    if (strcmp(v->path, real_path) == 0) {
      found = true;
      break;
    }
  }
  pthread_mutex_unlock(&deny_cache_lock);
  return found;
}

// Cache a deny for `real_path` with the short TTL. A repeated path refreshes
// its existing entry, and an expired entry's slot is reused, so a hot path
// retried in a loop cannot exhaust the table. If the table is still full the
// store is dropped silently — a dropped cache entry only costs an extra RPC,
// never a wrong verdict.
static void deny_cache_store(const char *real_path) {
  time_t now = deny_now_seconds();
  pthread_mutex_lock(&deny_cache_lock);
  struct deny_entry *slot = NULL;
  for (int i = 0; i < deny_cache_count; i++) {
    struct deny_entry *v = &deny_cache[i];
    if (strcmp(v->path, real_path) == 0) {
      slot = v; // same path: refresh in place
      break;
    }
    if (slot == NULL && now >= v->expiry)
      slot = v; // remember the first expired slot for reuse
  }
  if (slot == NULL && deny_cache_count < DENY_CACHE_MAX)
    slot = &deny_cache[deny_cache_count++];
  if (slot != NULL) {
    strncpy(slot->path, real_path, PATH_MAX - 1);
    slot->path[PATH_MAX - 1] = '\0';
    slot->expiry = now + PROMPT_DENY_TTL_SECONDS;
  }
  pthread_mutex_unlock(&deny_cache_lock);
}

// Append `src` to `dst` (a fixed buffer of size `cap`, NUL-terminated),
// JSON-escaping the only two characters that can appear unescaped in a path
// and break the wire format: backslash and double-quote. Truncates safely if
// the buffer fills. Used to build audit records by hand (no JSON library).
static void json_append_escaped(char *dst, size_t cap, const char *src) {
  size_t len = strlen(dst);
  for (const char *p = src; *p != '\0' && len + 2 < cap; p++) {
    if (*p == '\\' || *p == '"') {
      dst[len++] = '\\';
    }
    dst[len++] = *p;
  }
  dst[len] = '\0';
}

// Resolve the running executable's realpath into `out` (size PATH_MAX),
// best-effort: out[0] == '\0' on any failure. Linux exposes the executable as
// the /proc/self/exe symlink; macOS has no /proc, so ask dyld for the image
// path and canonicalize that. Used for the audit record's attribution column.
static void current_exe_realpath(char *out) {
  out[0] = '\0';
#ifdef linux
  if (realpath("/proc/self/exe", out) == NULL)
    out[0] = '\0';
#else
  char exe_path[PATH_MAX];
  uint32_t exe_size = sizeof(exe_path);
  if (_NSGetExecutablePath(exe_path, &exe_size) != 0 ||
      realpath(exe_path, out) == NULL)
    out[0] = '\0';
#endif
}

// The current sandbox level as the mode string recorded in audit records.
// prompt is enforce-with-prompt_mode rather than a distinct level, so it is
// checked first.
static const char *sandbox_level_name(void) {
  if (prompt_mode)
    return "prompt";
  switch (sandbox_level) {
  case SANDBOX_LEVEL_WARN:
    return "warn";
  case SANDBOX_LEVEL_ENFORCE:
    return "enforce";
  case SANDBOX_LEVEL_PURE:
    return "pure";
  default:
    return "off";
  }
}

// ===========================================================================
// Audit store.
//
// Every report the engine emits — warn-mode reports and enforce/prompt
// denials,
// for file accesses, directory enumerations, and network connects alike — is
// also appended as one NDJSON record to
// $FLOX_SANDBOX_GRANTS_DIR/audit.ndjson, so a denial is queryable after the
// session with `flox sandbox audit`. Allowed accesses are never recorded.
//
// The hooks ride the existing once-per-key dedup (should_warn_for_path), so
// volume is bounded to one record per path per process. Writes are
// best-effort: a failure (read-only .flox, full disk) never changes a
// verdict and never raises an error. Builds never set
// FLOX_SANDBOX_GRANTS_DIR, so the hook is inert for them.
// ===========================================================================
static void audit_append(const char *kind, const char *op, const char *path,
                         const char *verdict) {
  if (sandbox_grants_dir == NULL || sandbox_grants_dir[0] == '\0')
    return;

  char audit_path[PATH_MAX];
  if (snprintf(audit_path, sizeof(audit_path), "%s/audit.ndjson",
               sandbox_grants_dir) >= (int)sizeof(audit_path))
    return;

  char exe_real[PATH_MAX];
  current_exe_realpath(exe_real);

  // Build the record by hand (no JSON library), escaping the only two
  // characters that can break the wire format in a path. The buffer leaves
  // room for the fixed fields plus a fully-escaped path and exe.
  char line[PATH_MAX * 3 + 256];
  snprintf(line, sizeof(line),
           "{\"ts\":%ld,\"mode\":\"%s\",\"kind\":\"%s\",\"op\":\"%s\","
           "\"path\":\"",
           (long)time(NULL), sandbox_level_name(), kind, op);
  json_append_escaped(line, sizeof(line), path);
  size_t len = strlen(line);
  snprintf(line + len, sizeof(line) - len,
           "\",\"verdict\":\"%s\",\"pid\":%d,\"exe\":\"", verdict, getpid());
  json_append_escaped(line, sizeof(line), exe_real);
  len = strlen(line);
  if (len + 3 >= sizeof(line))
    return; // no room to terminate the record; drop rather than corrupt
  line[len++] = '"';
  line[len++] = '}';
  line[len++] = '\n';

  // Append through the real syscalls: with in_sandbox set our own
  // interceptors pass straight through (mirroring how prompt_broker does its
  // socket I/O). Saved and restored so the hook behaves identically if it is
  // ever reached from a path that did not already set the guard.
  int saved_in_sandbox = in_sandbox;
  in_sandbox = 1;
  int fd = open(audit_path, O_WRONLY | O_APPEND | O_CREAT, 0600);
  if (fd >= 0) {
    // One write per record: O_APPEND keeps concurrent appenders from
    // interleaving within a line at this size.
    ssize_t written = write(fd, line, len);
    (void)written;
    close(fd);
  }
  in_sandbox = saved_in_sandbox;
}

// Apply the per-level out-of-policy verdict and return whether to permit the
// access. This is the shared tail for an access that has already been judged
// out of policy: a regular out-of-closure file access, an
// activation-denied sensitive path, or an activation create whose parent
// directory is not in policy. `display` is the user-facing path string;
// `dedup_key` is the key used to warn/deny at most once (the resolved realpath
// for an existing path, or the opened pathname for a create — cwd-absolutized
// when the sensitive set flagged the create). `suffix` is an
// optional parenthetical reason appended to the message (e.g. " (sensitive)"),
// or "" for none. `raw` is the path as originally opened (no longer sent to
// the broker, which receives only the realpath; kept so call sites stay
// stable while the wire protocol evolves).
//
//   warn         -> warn once, permit (return true)
//   prompt       -> consult the deny cache, else ask the broker; on deny
//                   emit the receipt (once per path), on allow stay silent;
//                   refuse on any deny (return false -> EACCES). With no
//                   reachable broker, an activation emits the fail-closed
//                   receipt (once per path) and refuses, while a build falls
//                   through to plain enforce.
//   enforce/pure -> report once, refuse (return false -> EACCES). A policy
//                   denial is NEVER fatal: a shell builtin redirect performs
//                   its open inside the interactive shell process itself, so
//                   exit(1) would kill the user's shell.
static bool out_of_policy_verdict(const char *display, const char *dedup_key,
                                  const char *raw, const char *suffix) {
  (void)raw;
  // The op recorded in receipts and audit records. A directory enumeration
  // arrives here as a read of the directory path (in_write_access unset).
  const char *op = in_write_access ? "write" : "read";

  if (sandbox_level == SANDBOX_LEVEL_WARN) {
    // warn mode: report the out-of-policy access, but only once per key — a
    // process that touches the same undeclared path repeatedly should produce
    // a single warning, not one per access.
    if (should_warn_for_path(dedup_key)) {
      warn("%s is not in the sandbox%s", display, suffix);
      audit_append("fs", op, dedup_key, "warned");
    }
    return true;
  }
  if (prompt_mode) {
    // 1. Deny cache first (cheapest path; zero RPCs). A still-fresh deny
    //    short-circuits the broker entirely; allow verdicts were already
    //    cached client-side by prompt_broker (allow_globs_add) and matched
    //    upstream in sandbox_check_path. dedup_key is the resolved realpath
    //    for an existing path, which is exactly what the cache keys on.
    if (deny_cache_hit(dedup_key))
      return false;

    // 2. Ask the broker.
    unsigned long req = 0;
    int decision = prompt_broker(dedup_key, &req);

    if (decision == PROMPT_ALLOW) {
      // Allow: permit silently (no receipt on allow). prompt_broker already
      // remembered the path/pattern, so the subtree is silenced with no
      // further RPCs.
      return true;
    }

    if (decision == PROMPT_DENY) {
      // Deny: cache the exact path with a short TTL so a retry after the TTL
      // re-asks (and can pick up a later grant), then emit the receipt once
      // per path. The activation broker queues the request and replies
      // `deny <req>`, so the receipt names the review id; an interactive
      // build broker replies the bare form (the user already saw and
      // answered the prompt). The false propagates to each interceptor's
      // errno=EACCES branch, so the caller sees a clean permission error and
      // continues — never exit(1).
      deny_cache_store(dedup_key);
      if (should_warn_for_path(dedup_key)) {
        if (req > 0) {
          denied("%s %s (not in policy)", op, display);
          denied("queued as req %lu — approve outside: flox sandbox", req);
        } else {
          denied("%s denied by sandbox prompt", display);
        }
        audit_append("fs", op, dedup_key, "denied");
      }
      return false;
    }

    // PROMPT_ERROR: no broker, or the exchange failed. For an ACTIVATION
    // (allow_foreign_exe set) fail closed with a distinct receipt — a dead
    // executive must degrade prompt to enforce-with-better-errors, never
    // silently unsandbox or kill the user's shell. Cache nothing: the broker
    // may come back, and a later retry should re-ask rather than stay stuck
    // on a cached error. For a BUILD, fall through to plain enforce below
    // (prompt with no broker is just enforce).
    if (allow_foreign_exe) {
      if (should_warn_for_path(dedup_key)) {
        _error("prompt broker unreachable; denying %s of %s (fail-closed; "
               "restart the activation to restore approvals)",
               op, display);
        audit_append("fs", op, dedup_key, "fail-closed");
      }
      return false;
    }
  }
  // enforce / pure: report and refuse with a clean error. The caller maps the
  // false return to errno=EACCES and returns -1/NULL, so the access fails like
  // any permission error and the calling process continues. A policy denial
  // must never exit(1): a shell builtin redirect (echo x >> secret) performs
  // the open inside the interactive shell process, so aborting would terminate
  // the user's shell. Reported once per key, mirroring warn.
  if (should_warn_for_path(dedup_key)) {
    _error("%s is not in the sandbox%s", display, suffix);
    audit_append("fs", op, dedup_key, "denied");
  }
  fflush(stderr);
  return false;
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

  // From here on out, operate on realpath. If a file doesn't exist then it has
  // no realpath. This must be a local (stack) buffer: a shared `static` here
  // was a data race, since concurrent callers resolving different paths would
  // overwrite each other between this call and the closure/allow-list checks
  // below.
  char real_path[PATH_MAX];
  if (realpath(pathname, real_path) == NULL) {
    // The path does not exist. For a BUILD (allow_foreign_exe unset) keep the
    // historical behaviour: permit it and let ENOENT surface naturally — a
    // build legitimately creates many new files, and read-of-nonexistent must
    // be allowed so the caller observes the real error.
    //
    // For an ACTIVATION that is creating a file (allow_foreign_exe set and the
    // access is a write), a nonexistent path is a NEW FILE. Letting every such
    // create through means an agent can write `~/newfile` or overwrite outside
    // the project at will. Instead, judge the create by the nearest EXISTING
    // ancestor directory's policy (see create_parent_in_policy): a create under
    // an in-policy directory — including into a not-yet-existing subtree, e.g.
    // git's `.git/objects/<fanout>/tmp_obj_*` — is fine; a create anywhere else
    // is out of policy and gets the per-level verdict. A read of a nonexistent
    // path, or a create whose ancestor IS in policy, is permitted as before.
    //
    // Walking up to the nearest existing ancestor (rather than requiring the
    // immediate parent to already exist) is necessary for real workloads: git,
    // mkdir -p, and similar tools create files inside directories they create
    // on the fly, so an immediate-parent rule would deny ordinary in-project
    // writes. The threat model is preserved — the nearest existing ancestor of
    // `~/newfile` is `$HOME`, which is out of policy, so the create is denied.
    if (allow_foreign_exe && in_write_access) {
      // The exe-identity report is a no-op here (allow_foreign_exe is set),
      // so it is not called; only file access is being mediated.
      //
      // A NEW sensitive file (writing a fresh ~/.aws/credentials, or a project
      // .env) is denied even when its parent directory is in policy. Without
      // this, the sensitive-file protection that covers existing files (below)
      // would have an obvious hole for first-time creation: an agent could
      // write a credential file into the project tree. The target has no
      // realpath yet, and the sensitive globs (`**`-anchored or `~/`-expanded)
      // only match ABSOLUTE strings: a bare relative create — `cd project &&
      // echo x > .env` opens ".env" — slips past a raw match because the
      // pattern's literal '/' must appear in the string under fnmatch with
      // flags=0. So absolutize a relative pathname against the cwd, exactly
      // as the open() being guarded resolves it, and match the sensitive set
      // against that candidate. The joined candidate is not canonicalized
      // (a nonexistent leaf has no realpath, so "." / ".." components are
      // kept); the `**` patterns still match through them, and full
      // canonicalization is outside the cooperative-tools threat model. On
      // any cwd/overflow trouble fall back to the raw path, which preserves
      // the absolute-path behaviour. The candidate also serves as display and
      // dedup key so the report names the actual target; the RPC raw field
      // stays the path as opened.
      char abs_buf[PATH_MAX];
      const char *candidate = pathname;
      if (pathname[0] != '/') {
        char cwd_real[PATH_MAX];
        if (realpath(".", cwd_real) != NULL &&
            (size_t)snprintf(abs_buf, sizeof(abs_buf), "%s/%s", cwd_real,
                             pathname) < sizeof(abs_buf))
          candidate = abs_buf;
      }
      if (path_is_sensitive(candidate))
        return out_of_policy_verdict(candidate, candidate, pathname,
                                     " (sensitive)");
      // The non-sensitive deny uses the absolutized candidate too: the broker
      // request, the deny cache, the receipt dedup, and the pending review
      // entry all key on it, and only an absolute path can match an absolute
      // grant glob (so 'approve outside, then retry' actually redeems).
      if (!create_parent_in_policy(pathname))
        return out_of_policy_verdict(candidate, candidate, pathname, "");
    }
    return true;
  }

  // Grants-dir write guard (prompt only). The grants directory is seeded into
  // the
  // project's allow set so reads stay quiet, but a write there would let a
  // process edit its own future-session approvals. In prompt mode, route
  // writes under the grants dir through the prompt flow instead of letting
  // the allow shortcuts wave them through. Reads are unaffected, and without
  // a configured grants dir the guard is inert. When the guard fires we skip
  // the allow-list shortcuts and fall through to the out-of-policy tail. The
  // grants-dir realpath is resolved lazily on first need here
  // (short-circuited for the common not-prompt / not-write case), never in
  // init.
  bool grants_dir_write_guard =
      prompt_mode && in_write_access &&
      path_under(real_path, grants_dir_real, grants_dir_real_length());
  if (!grants_dir_write_guard) {
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
  }

  // If the running executable is itself outside the closure, report it once
  // before any per-path message so the user sees the root cause first. This
  // is ALWAYS a warning, never fatal on its own: it is context for the
  // per-path verdict that follows, and that verdict is a graceful EACCES
  // under enforce/pure (see out_of_policy_verdict). It is build-only — an
  // activation returns early from the check via allow_foreign_exe, since
  // running host tools from outside the closure is the activation's whole
  // point.
  maybe_report_process_outside_closure();

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

  // An open/openat with O_DIRECTORY (in_dir_probe) cannot read file contents —
  // the kernel returns ENOTDIR for non-directory paths, so no out-of-closure
  // data escapes. Treat it as a probe ("looking around"), warn but permit even
  // under enforce.
  if (in_dir_probe) {
    if (should_warn_for_path(real_path))
      warn("%s is outside the closure but permitted (directory probe)",
           display);
    return true;
  }

  // Directory accesses are "looking around" rather than reading out-of-closure
  // contents, so for a BUILD they are permitted even under enforce — with a
  // warning, but only the first time we see each directory (builds list the
  // same directory many times, which otherwise floods the log).
  //
  // For an ACTIVATION (allow_foreign_exe set) a directory ENUMERATION
  // (in_dir_read: opendir/fdopendir) is different: it hands the caller every
  // entry name in the directory, so listing an out-of-policy directory leaks
  // its contents' names. Treat the enumeration as a READ of the directory
  // path and route it through the shared per-level verdict (warn once +
  // permit; graceful EACCES under enforce; deny + queue under prompt) so
  // report — and any future audit hook on the shared helper — sees it. Gated
  // on allow_foreign_exe so build behaviour is byte-identical: the build-mode
  // warn-permit below is unchanged.
  if (is_directory(real_path)) {
    if (allow_foreign_exe && in_dir_read)
      return out_of_policy_verdict(display, real_path, pathname,
                                   " (directory listing)");
    if (should_warn_for_path(real_path))
      warn("%s is outside the closure but permitted (directory listing)",
           display);
    return true;
  }
  // A directory-read of a NON-directory cannot enumerate anything — the
  // kernel returns ENOTDIR regardless — so permit it and let the caller
  // observe the natural error instead of a policy denial.
  if (in_dir_read)
    return true;
  // Sensitive set (activation only). For an activation (allow_foreign_exe set),
  // credential and secret files (~/.ssh, ~/.aws, ~/.netrc, **/.env, ...) are
  // denied even under enforce — and crucially BEFORE the $HOME-dotfile
  // carve-out below, which would otherwise wave ~/.ssh/* and ~/.aws/* through.
  // This runs after the explicit allow checks above, so an explicit user grant
  // (FLOX_SANDBOX_ALLOW) of a sensitive path still wins; only the implicit
  // dotfile blanket is overridden. The metadata-only carve-outs above
  // (readlink, directory probe, directory listing) stay permitted — those are
  // "looking around", not reading secret contents.
  //
  // Gated on allow_foreign_exe so a build never consults the sensitive set:
  // build-sandbox behaviour is byte-identical.
  if (allow_foreign_exe && path_is_sensitive(real_path)) {
    // maybe_report_process_outside_closure() already ran above and is a no-op
    // under allow_foreign_exe; only file access is mediated here.
    return out_of_policy_verdict(display, real_path, pathname, " (sensitive)");
  }
  // User config dotfiles under $HOME are permitted even under enforce, but
  // flagged (and followed by a one-time hint), so the developer knows the build
  // still depends on $HOME state on the path to full purity. As with directory
  // listings, warn only the first time we see each dotfile — builds re-read the
  // same config files (~/.gitconfig, ~/.npmrc, ...) repeatedly.
  //
  // Under a prompt-mode ACTIVATION this carve-out is deliberately skipped:
  // the dotfile blanket is a build-purity convenience that is exactly
  // backwards for an interactive agent threat model (it would wave through
  // ~/.aws/credentials, ~/.ssh/*, ...), so there — and only there — dotfiles
  // route through the prompt flow below. A prompt-mode BUILD keeps the
  // carve-out, like every other build level. The metadata-only carve-outs
  // above (readlink, directory probe, directory listing) stay permitted for
  // every level.
  if (!(prompt_mode && allow_foreign_exe) && is_home_dotfile(real_path)) {
    if (should_warn_for_path(real_path)) {
      warn("%s is outside the closure but permitted as a $HOME dotfile",
           display);
      home_dotfile_hint();
    }
    return true;
  }
  // Out of policy. Apply the per-level verdict (warn permits; prompt consults
  // the deny cache then the broker, caching the result; enforce/pure refuses),
  // deduplicated on the resolved realpath. Prompt-mode consultation lives in
  // the shared tail — gated on prompt_mode (not merely the socket) so an
  // enforce-mode build sharing the process-wide socket env never prompts —
  // and the tail also serves creates, directory enumerations, sensitive
  // paths, and grants-guard writes, so every caller class gets the same
  // broker flow.
  return out_of_policy_verdict(display, real_path, pathname, "");
}

// Classify an open()/openat() as a write from its flags. Anything that is not
// purely read-only — write, read-write, or append — counts as a write. Used
// only to populate in_write_access (the audit op column, the create guard,
// and the grants-dir write guard); read-vs-write does not change whether an
// access is permitted.
static int open_is_write(int flags) {
  return (flags & O_ACCMODE) != O_RDONLY || (flags & O_APPEND) ? 1 : 0;
}

// Classify an fopen()/fopen64() mode string as a write. The C standard mode
// grammar marks a write whenever it contains 'w' (truncate), 'a' (append), or
// '+' (read-update / write-update). A bare "r"/"rb" is the only read-only
// form. Same purpose as open_is_write().
static int fopen_is_write(const char *mode) {
  if (mode == NULL)
    return 0;
  return strchr(mode, 'w') != NULL || strchr(mode, 'a') != NULL ||
                 strchr(mode, '+') != NULL
             ? 1
             : 0;
}

// ===========================================================================
// Network egress mediation.
//
// The filesystem engine above warns or denies out-of-policy file access; this
// section applies the same gradient to outbound TCP connections. connect() is
// the single choke point for TCP egress, so intercepting it is enough to
// mediate every cooperative dynamically-linked client. getaddrinfo() is
// observed (never blocked) purely to attach a human-readable hostname to the
// IPs a later connect() targets — best-effort metadata for messages, never a
// security boundary.
//
// The policy lives in FLOX_SANDBOX_ALLOW_NET, a space-separated list whose
// entries are matched against the connection destination:
//   - "ip"            exact IPv4/IPv6 literal
//   - "ip/cidr"       CIDR block (IPv4 or IPv6)
//   - "ip:port" / "ip/cidr:port"  as above, restricted to one port
//   - "host" / "host:port"        matches if getaddrinfo observed this IP
//                                 resolving from that hostname (best effort)
// Loopback (127.0.0.0/8, ::1) and AF_UNIX sockets are always allowed and never
// consult the policy. AF_UNIX in particular must never be mediated: it is the
// transport for the broker itself and for process-compose, and blocking it
// would break the sandbox's own plumbing.
// ===========================================================================

// Best-effort IP -> hostname attribution cache, populated by the getaddrinfo
// interceptor and consulted by sandbox_check_connect to (a) match hostname
// allow-net entries and (b) name the destination in messages. It is a small
// fixed-size ring: when full, the oldest entry is overwritten. Mutex-guarded
// like warned_paths; this is metadata only, so a miss (or an overwrite under
// churn) merely yields a bare-IP message, never a wrong verdict.
#define NET_NAME_CACHE_MAX 64
#define NET_IP_STRLEN INET6_ADDRSTRLEN
#define NET_HOST_STRLEN 256
struct net_name_entry {
  char ip[NET_IP_STRLEN];
  char host[NET_HOST_STRLEN];
};
static struct net_name_entry net_name_cache[NET_NAME_CACHE_MAX];
static int net_name_cache_count = 0; // number of valid entries (<= max)
static int net_name_cache_next = 0;  // next slot to overwrite once full
static pthread_mutex_t net_name_cache_lock = PTHREAD_MUTEX_INITIALIZER;

// Record that `ip` most recently resolved from `host`. If `ip` is already
// present its hostname is refreshed; otherwise a new entry is inserted (ring
// overwrite when full). Called only from the getaddrinfo interceptor with
// in_sandbox==1.
static void net_name_cache_put(const char *ip, const char *host) {
  if (ip == NULL || host == NULL || ip[0] == '\0' || host[0] == '\0')
    return;
  pthread_mutex_lock(&net_name_cache_lock);
  for (int i = 0; i < net_name_cache_count; i++) {
    if (strcmp(net_name_cache[i].ip, ip) == 0) {
      strncpy(net_name_cache[i].host, host, NET_HOST_STRLEN - 1);
      net_name_cache[i].host[NET_HOST_STRLEN - 1] = '\0';
      pthread_mutex_unlock(&net_name_cache_lock);
      return;
    }
  }
  int slot;
  if (net_name_cache_count < NET_NAME_CACHE_MAX)
    slot = net_name_cache_count++;
  else {
    slot = net_name_cache_next;
    net_name_cache_next = (net_name_cache_next + 1) % NET_NAME_CACHE_MAX;
  }
  strncpy(net_name_cache[slot].ip, ip, NET_IP_STRLEN - 1);
  net_name_cache[slot].ip[NET_IP_STRLEN - 1] = '\0';
  strncpy(net_name_cache[slot].host, host, NET_HOST_STRLEN - 1);
  net_name_cache[slot].host[NET_HOST_STRLEN - 1] = '\0';
  pthread_mutex_unlock(&net_name_cache_lock);
}

// Look up the most-recent hostname for `ip`, copying it into `host_out` (size
// NET_HOST_STRLEN). Returns true on a hit. Best-effort: a miss is normal (the
// client may have resolved via a path we did not observe, or used a literal
// IP) and simply yields a nameless message.
static bool net_name_cache_get(const char *ip, char *host_out) {
  bool found = false;
  pthread_mutex_lock(&net_name_cache_lock);
  for (int i = 0; i < net_name_cache_count; i++) {
    if (strcmp(net_name_cache[i].ip, ip) == 0) {
      strncpy(host_out, net_name_cache[i].host, NET_HOST_STRLEN - 1);
      host_out[NET_HOST_STRLEN - 1] = '\0';
      found = true;
      break;
    }
  }
  pthread_mutex_unlock(&net_name_cache_lock);
  return found;
}

// Parsed FLOX_SANDBOX_ALLOW_NET, one entry per token, built once under
// allow_net_once. Mirrors the fs allow-list shape (256-entry cap, read-only
// after init).
#define FLOX_SANDBOX_ALLOW_NET_MAXENTRIES 256
#define FLOX_SANDBOX_ALLOW_NET_MAXLEN (16 * 1024)

// One allow-net entry. `is_cidr`/`is_ip` distinguish a numeric IP/CIDR rule
// from a hostname rule. For IP/CIDR entries, `addr` holds the parsed network
// (4 or 16 bytes) and `prefix_bits` the CIDR width; for hostname entries,
// `host` holds the name. `port` is 0 for "any port", else the single allowed
// port. `family` is AF_INET or AF_INET6 for IP entries.
struct allow_net_entry {
  bool is_ip;                 // entry is an IP or CIDR (vs a hostname)
  int family;                 // AF_INET / AF_INET6 (IP entries only)
  unsigned char addr[16];     // network bytes (IP entries only)
  int prefix_bits;            // CIDR prefix width (IP entries only)
  char host[NET_HOST_STRLEN]; // hostname (hostname entries only)
  int port;                   // 0 = any port, else the only permitted port
};
static pthread_once_t allow_net_once = PTHREAD_ONCE_INIT;
static char allow_net_buf[FLOX_SANDBOX_ALLOW_NET_MAXLEN];
static struct allow_net_entry allow_net[FLOX_SANDBOX_ALLOW_NET_MAXENTRIES];
static int allow_net_count = 0;

// Split a "host[:port]" token into host and port. Returns the port (0 if
// absent) and writes the host portion into `host_out` (size host_out_len).
// For IPv6 literals the address may itself contain ':', so a port suffix is
// only recognized when the address is bracketed ("[::1]:443") or the token has
// exactly one ':' (an IPv4/host form). Bracketed IPv6 has its brackets
// stripped from host_out.
static int split_host_port(const char *token, char *host_out,
                           size_t host_out_len) {
  // Bracketed form: [addr] or [addr]:port.
  if (token[0] == '[') {
    const char *close = strchr(token, ']');
    if (close != NULL) {
      size_t hlen = (size_t)(close - token - 1);
      if (hlen >= host_out_len)
        hlen = host_out_len - 1;
      memcpy(host_out, token + 1, hlen);
      host_out[hlen] = '\0';
      if (close[1] == ':')
        return atoi(close + 2);
      return 0;
    }
  }
  // Unbracketed: a single ':' is a port separator (IPv4 or hostname). More
  // than one ':' means a bare IPv6 literal with no port.
  const char *first = strchr(token, ':');
  if (first != NULL && strchr(first + 1, ':') == NULL) {
    size_t hlen = (size_t)(first - token);
    if (hlen >= host_out_len)
      hlen = host_out_len - 1;
    memcpy(host_out, token, hlen);
    host_out[hlen] = '\0';
    return atoi(first + 1);
  }
  // No port: copy the whole token as the host.
  strncpy(host_out, token, host_out_len - 1);
  host_out[host_out_len - 1] = '\0';
  return 0;
}

// Parse one allow-net token into `entry`. Recognizes "ip", "ip/cidr",
// "host" with optional ":port" suffix. Returns true on success.
static bool parse_allow_net_entry(const char *token,
                                  struct allow_net_entry *entry) {
  memset(entry, 0, sizeof(*entry));

  char host_part[NET_HOST_STRLEN];
  entry->port = split_host_port(token, host_part, sizeof(host_part));

  // Split a trailing "/cidr" off the host part.
  int prefix_bits = -1;
  char *slash = strchr(host_part, '/');
  if (slash != NULL) {
    prefix_bits = atoi(slash + 1);
    *slash = '\0';
  }

  // Try to parse the host part as a numeric IPv4 or IPv6 address.
  unsigned char buf4[4];
  unsigned char buf16[16];
  if (inet_pton(AF_INET, host_part, buf4) == 1) {
    entry->is_ip = true;
    entry->family = AF_INET;
    memcpy(entry->addr, buf4, 4);
    entry->prefix_bits =
        (prefix_bits >= 0 && prefix_bits <= 32) ? prefix_bits : 32;
    return true;
  }
  if (inet_pton(AF_INET6, host_part, buf16) == 1) {
    entry->is_ip = true;
    entry->family = AF_INET6;
    memcpy(entry->addr, buf16, 16);
    entry->prefix_bits =
        (prefix_bits >= 0 && prefix_bits <= 128) ? prefix_bits : 128;
    return true;
  }
  // Otherwise treat it as a hostname rule. A "/cidr" on a hostname is
  // meaningless; we ignore it (already stripped).
  if (host_part[0] == '\0')
    return false;
  entry->is_ip = false;
  strncpy(entry->host, host_part, NET_HOST_STRLEN - 1);
  entry->host[NET_HOST_STRLEN - 1] = '\0';
  return true;
}

static void allow_net_init(void) {
  const char *env = getenv("FLOX_SANDBOX_ALLOW_NET");
  if (env == NULL)
    return;
  if (strlen(env) >= sizeof(allow_net_buf)) {
    _error("FLOX_SANDBOX_ALLOW_NET is too long, truncating to %zu characters",
           sizeof(allow_net_buf) - 1);
    fflush(stderr);
  }
  strncpy(allow_net_buf, env, sizeof(allow_net_buf) - 1);
  allow_net_buf[sizeof(allow_net_buf) - 1] = '\0';

  char *saveptr = NULL;
  char *token = strtok_r(allow_net_buf, " ", &saveptr);
  while (token != NULL) {
    if (allow_net_count >= FLOX_SANDBOX_ALLOW_NET_MAXENTRIES) {
      _error("FLOX_SANDBOX_ALLOW_NET has too many entries, using the first %d",
             FLOX_SANDBOX_ALLOW_NET_MAXENTRIES);
      fflush(stderr);
      break;
    }
    if (parse_allow_net_entry(token, &allow_net[allow_net_count])) {
      debug("FLOX_SANDBOX_ALLOW_NET entry[%d] = %s", allow_net_count, token);
      allow_net_count++;
    } else {
      _error("FLOX_SANDBOX_ALLOW_NET: ignoring unparseable entry '%s'", token);
      fflush(stderr);
    }
    token = strtok_r(NULL, " ", &saveptr);
  }
}

// Return true if the `family`/`addr` destination falls inside the CIDR block
// described by `entry` (same family). Compares `prefix_bits` leading bits,
// byte by byte then a partial final byte.
static bool cidr_match(const struct allow_net_entry *entry, int family,
                       const unsigned char *addr) {
  if (entry->family != family)
    return false;
  int bits = entry->prefix_bits;
  int full_bytes = bits / 8;
  int rem_bits = bits % 8;
  if (memcmp(entry->addr, addr, (size_t)full_bytes) != 0)
    return false;
  if (rem_bits == 0)
    return true;
  unsigned char mask = (unsigned char)(0xff << (8 - rem_bits));
  return (entry->addr[full_bytes] & mask) == (addr[full_bytes] & mask);
}

// True if the destination ip string `ip` resolved (per the getaddrinfo cache)
// from the hostname `entry->host`. Best effort: a connect to a literal IP, or
// to a host resolved through a path we did not observe, will not match a
// hostname rule — only exact-IP/CIDR rules cover those.
static bool host_entry_matches(const struct allow_net_entry *entry,
                               const char *ip) {
  char cached_host[NET_HOST_STRLEN];
  if (!net_name_cache_get(ip, cached_host))
    return false;
  return strcasecmp(cached_host, entry->host) == 0;
}

// Decide whether a connection to `family`/`addr` (raw network bytes) on `port`
// (host order) is permitted by FLOX_SANDBOX_ALLOW_NET. `ip` is the same
// address already stringified (for hostname-cache lookups). Loopback is the
// caller's responsibility (checked before this).
static bool net_dest_allowed(int family, const unsigned char *addr,
                             const char *ip, int port) {
  pthread_once(&allow_net_once, allow_net_init);
  for (int i = 0; i < allow_net_count; i++) {
    const struct allow_net_entry *entry = &allow_net[i];
    // A port-qualified rule only matches that port; a port-0 rule matches any.
    if (entry->port != 0 && entry->port != port)
      continue;
    if (entry->is_ip) {
      if (cidr_match(entry, family, addr))
        return true;
    } else {
      if (host_entry_matches(entry, ip))
        return true;
    }
  }
  return false;
}

// Recognize loopback destinations (always allowed, never mediated): IPv4
// 127.0.0.0/8 and IPv6 ::1. Operates on raw network bytes.
static bool is_loopback(int family, const unsigned char *addr) {
  if (family == AF_INET)
    return addr[0] == 127; // 127.0.0.0/8
  if (family == AF_INET6) {
    // ::1 — fifteen zero bytes then a single 1.
    static const unsigned char v6_loopback[16] = {0, 0, 0, 0, 0, 0, 0, 0,
                                                  0, 0, 0, 0, 0, 0, 0, 1};
    if (memcmp(addr, v6_loopback, 16) == 0)
      return true;
    // ::ffff:127.0.0.0/8 — IPv4-mapped loopback.
    static const unsigned char v4mapped_prefix[12] = {0, 0, 0, 0, 0,    0,
                                                      0, 0, 0, 0, 0xff, 0xff};
    if (memcmp(addr, v4mapped_prefix, 12) == 0 && addr[12] == 127)
      return true;
  }
  return false;
}

// Extract the destination family, raw address bytes, and port from a
// connect() sockaddr. Returns true for AF_INET / AF_INET6 (the families we
// mediate), writing the address into `addr_out` (>= 16 bytes), the family into
// `*family_out`, and the host-order port into `*port_out`. Returns false for
// AF_UNIX and every other family — those are never mediated (Unix sockets are
// local IPC, used by the broker and process-compose; blocking them would break
// the sandbox's own plumbing).
static bool extract_dest(const struct sockaddr *sa, socklen_t addrlen,
                         int *family_out, unsigned char *addr_out,
                         int *port_out) {
  if (sa == NULL)
    return false;
  if (sa->sa_family == AF_INET) {
    if (addrlen < (socklen_t)sizeof(struct sockaddr_in))
      return false;
    const struct sockaddr_in *sin = (const struct sockaddr_in *)sa;
    *family_out = AF_INET;
    memcpy(addr_out, &sin->sin_addr, 4);
    // Network-order u16 -> host order without ntohs (which glibc inlines).
    unsigned short netport = sin->sin_port;
    *port_out = ((netport & 0xff) << 8) | ((netport >> 8) & 0xff);
    return true;
  }
  if (sa->sa_family == AF_INET6) {
    if (addrlen < (socklen_t)sizeof(struct sockaddr_in6))
      return false;
    const struct sockaddr_in6 *sin6 = (const struct sockaddr_in6 *)sa;
    *family_out = AF_INET6;
    memcpy(addr_out, &sin6->sin6_addr, 16);
    unsigned short netport = sin6->sin6_port;
    *port_out = ((netport & 0xff) << 8) | ((netport >> 8) & 0xff);
    return true;
  }
  return false;
}

// Apply the network-egress policy to one connect() destination.
//
// Returns true to permit the connection (the interceptor proceeds to the real
// connect), false to refuse it (the interceptor sets errno=ECONNREFUSED and
// returns -1 — a clean connection failure the application can handle, never an
// exit()). AF_UNIX and unparseable addresses always return true: they are not
// mediated. Loopback always returns true. Off mode returns true. Otherwise the
// destination is matched against FLOX_SANDBOX_ALLOW_NET:
//   - warn: out-of-policy destinations are reported once per dest, permitted.
//   - enforce/pure: out-of-policy destinations are refused (ECONNREFUSED).
//   - prompt: there is no network broker yet (it lands in a later batch), so
//     applies enforce semantics for the network — refuse out-of-policy with a
//     clean ECONNREFUSED rather than inventing a net receipt the broker will
//     define. The filesystem prompt flow is unaffected.
static bool sandbox_check_connect(const struct sockaddr *sa,
                                  socklen_t addrlen) {
  ensure_init();
  if (sandbox_level == SANDBOX_LEVEL_OFF)
    return true;

  int family;
  unsigned char addr[16];
  int port;
  if (!extract_dest(sa, addrlen, &family, addr, &port))
    return true; // AF_UNIX and other families are never mediated.

  if (is_loopback(family, addr))
    return true; // loopback is always allowed, silently.

  // Stringify the destination once for messages and hostname matching.
  char ip[NET_IP_STRLEN] = "";
  if (inet_ntop(family, addr, ip, sizeof(ip)) == NULL)
    snprintf(ip, sizeof(ip), "?");

  if (net_dest_allowed(family, addr, ip, port))
    return true; // in policy: permit silently.

  // Out of policy. Attach a hostname if getaddrinfo observed one, so the
  // message names the destination the user recognizes.
  char host[NET_HOST_STRLEN];
  bool have_host = net_name_cache_get(ip, host);

  // The destination as recorded in audit records: the hostname when
  // getaddrinfo observed one (the name the user recognizes), else the bare
  // IP, with the port attached either way.
  char dest_audit[NET_HOST_STRLEN + 16];
  snprintf(dest_audit, sizeof(dest_audit), "%s:%d", have_host ? host : ip,
           port);

  if (sandbox_level == SANDBOX_LEVEL_WARN) {
    // Warn once per destination (ip:port), modeled on the fs per-path dedup so
    // a client that retries the same endpoint does not flood the log. The key
    // is "ip:port" so different ports to the same host each warn once.
    char dest_key[NET_IP_STRLEN + 16];
    snprintf(dest_key, sizeof(dest_key), "%s:%d", ip, port);
    if (should_warn_for_path(dest_key)) {
      if (have_host)
        warn("connect to %s:%d (%s) is not in the network policy", host, port,
             ip);
      else
        warn("connect to %s:%d is not in the network policy", ip, port);
      audit_append("net", "connect", dest_audit, "warned");
    }
    return true; // warn permits the connect.
  }

  // enforce / pure / prompt (no net broker yet): refuse with a clean
  // ECONNREFUSED. Report once per destination so a retrying client does not
  // spam, mirroring warn's dedup.
  char dest_key[NET_IP_STRLEN + 16];
  snprintf(dest_key, sizeof(dest_key), "%s:%d", ip, port);
  if (should_warn_for_path(dest_key)) {
    if (have_host)
      _error("connect to %s:%d (%s) is not in the network policy", host, port,
             ip);
    else
      _error("connect to %s:%d is not in the network policy", ip, port);
    audit_append("net", "connect", dest_audit, "denied");
    fflush(stderr);
  }
  return false;
}

// Observe a getaddrinfo() result set, recording each resolved IP -> the queried
// hostname into the attribution cache. Best-effort and message-only: it never
// blocks resolution and never affects a verdict. Called from the getaddrinfo
// interceptor after the real resolution, with in_sandbox==1.
static void net_observe_resolution(const char *node, struct addrinfo *res) {
  if (node == NULL || res == NULL)
    return;
  for (struct addrinfo *ai = res; ai != NULL; ai = ai->ai_next) {
    char ip[NET_IP_STRLEN];
    if (ai->ai_family == AF_INET &&
        ai->ai_addrlen >= (socklen_t)sizeof(struct sockaddr_in)) {
      const struct sockaddr_in *sin = (const struct sockaddr_in *)ai->ai_addr;
      if (inet_ntop(AF_INET, &sin->sin_addr, ip, sizeof(ip)) != NULL)
        net_name_cache_put(ip, node);
    } else if (ai->ai_family == AF_INET6 &&
               ai->ai_addrlen >= (socklen_t)sizeof(struct sockaddr_in6)) {
      const struct sockaddr_in6 *sin6 =
          (const struct sockaddr_in6 *)ai->ai_addr;
      if (inet_ntop(AF_INET6, &sin6->sin6_addr, ip, sizeof(ip)) != NULL)
        net_name_cache_put(ip, node);
    }
  }
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
  in_dir_probe = (flags & O_DIRECTORY) ? 1 : 0;
  in_write_access = open_is_write(flags);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_dir_probe = 0;
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
  in_dir_probe = (flags & O_DIRECTORY) ? 1 : 0;
  in_write_access = open_is_write(flags);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_dir_probe = 0;
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
  in_write_access = fopen_is_write(mode);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
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
  in_write_access = fopen_is_write(mode);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_fopen64(pathname, mode);
  errno = EACCES;
  return NULL;
}

// Interceptor for creat — equivalent to open(path, O_WRONLY|O_CREAT|O_TRUNC),
// so always a write. A tool that opens via creat() rather than open() must not
// slip past the sandbox.
int creat(const char *pathname, mode_t mode) {
  ensure_init();
  if (in_sandbox)
    return orig_creat(pathname, mode);
  in_sandbox = 1;
  in_write_access = 1;
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_creat(pathname, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for open64 — the _FILE_OFFSET_BITS=64 large-file variant of
// open(). 64-bit builds bind open64 instead of open, so without this their
// opens would never reach the policy. Identical logic to open().
int open64(const char *pathname, int flags, ...) {
  ensure_init();
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = va_arg(args, mode_t);
    va_end(args);
  }
  if (in_sandbox)
    return orig_open64(pathname, flags, mode);
  in_sandbox = 1;
  in_dir_probe = (flags & O_DIRECTORY) ? 1 : 0;
  in_write_access = open_is_write(flags);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_dir_probe = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_open64(pathname, flags, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for openat64 — the large-file variant of openat(). Same logic.
int openat64(int dirfd, const char *pathname, int flags, ...) {
  ensure_init();
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = va_arg(args, mode_t);
    va_end(args);
  }
  if (in_sandbox)
    return orig_openat64(dirfd, pathname, flags, mode);
  in_sandbox = 1;
  in_dir_probe = (flags & O_DIRECTORY) ? 1 : 0;
  in_write_access = open_is_write(flags);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_dir_probe = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_openat64(dirfd, pathname, flags, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for creat64 — the large-file variant of creat(). Same logic.
int creat64(const char *pathname, mode_t mode) {
  ensure_init();
  if (in_sandbox)
    return orig_creat64(pathname, mode);
  in_sandbox = 1;
  in_write_access = 1;
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_creat64(pathname, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for truncate — a path-addressed destructive write. ftruncate(fd,
// len) operates on an already-open fd (no path) and so cannot be mediated by
// path-based interposition; it is a documented residual limit, mitigated by
// the fact that the fd had to pass an open() check first.
int truncate(const char *path, off_t length) {
  ensure_init();
  if (in_sandbox)
    return orig_truncate(path, length);
  in_sandbox = 1;
  in_write_access = 1;
  bool allowed = sandbox_check_path(path);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_truncate(path, length);
  errno = EACCES;
  return -1;
}

// Interceptor for freopen — opens (and truncates) a path behind a FILE*, so it
// is a write whenever the mode is not pure-read. A NULL pathname reopens the
// existing stream in place (a mode change only); there is no path to mediate,
// so it is passed straight through.
FILE *freopen(const char *pathname, const char *mode, FILE *stream) {
  ensure_init();
  if (in_sandbox || pathname == NULL)
    return orig_freopen(pathname, mode, stream);
  in_sandbox = 1;
  in_write_access = fopen_is_write(mode);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_freopen(pathname, mode, stream);
  errno = EACCES;
  return NULL;
}

// Interceptor for freopen64 — the large-file variant of freopen(). Same logic.
FILE *freopen64(const char *pathname, const char *mode, FILE *stream) {
  ensure_init();
  if (in_sandbox || pathname == NULL)
    return orig_freopen64(pathname, mode, stream);
  in_sandbox = 1;
  in_write_access = fopen_is_write(mode);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_freopen64(pathname, mode, stream);
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

// Interceptor for opendir — the directory-enumeration choke point. `ls` and
// shell globs list directories via opendir()/readdir(); glibc's opendir()
// opens the directory through an internal __open*_nocancel symbol that never
// touches the exported open()/openat() PLT entries, so without this
// interceptor an enumeration reaches no check at all. The verdict is applied
// to the directory path as a read; a refusal is a graceful NULL + EACCES.
DIR *opendir(const char *name) {
  ensure_init();
  // Re-entrancy guard. Beyond the usual internal-call concern, this one is
  // load-bearing for correctness: is_directory() inside sandbox_check_path()
  // probes via opendir() itself, so without the short-circuit the check would
  // recurse straight back into the policy.
  if (in_sandbox)
    return orig_opendir(name);
  in_sandbox = 1;
  in_dir_read = 1;
  bool allowed = sandbox_check_path(name);
  in_dir_read = 0;
  in_sandbox = 0;
  if (allowed)
    return orig_opendir(name);
  errno = EACCES;
  return NULL;
}

// Interceptor for fdopendir — enumeration over an fd obtained from an earlier
// open(O_DIRECTORY) (the openat()+fdopendir() traversal style used by find
// and fts). The fd is mapped back to its directory path via /proc/self/fd; on
// mapping failure the call FAILS OPEN, because the fd can only have come from
// an open()/openat() that already passed the O_DIRECTORY probe check.
DIR *fdopendir(int fd) {
  ensure_init();
  if (in_sandbox)
    return orig_fdopendir(fd);
  in_sandbox = 1;
  bool allowed = true;
  char fd_link[64];
  char fd_path[PATH_MAX];
  snprintf(fd_link, sizeof(fd_link), "/proc/self/fd/%d", fd);
  // orig_readlink, not readlink: the exported interceptor would only bounce
  // through its own in_sandbox guard to land in the same place.
  ssize_t n = orig_readlink(fd_link, fd_path, sizeof(fd_path) - 1);
  if (n > 0) {
    fd_path[n] = '\0';
    in_dir_read = 1;
    allowed = sandbox_check_path(fd_path);
    in_dir_read = 0;
  }
  in_sandbox = 0;
  if (allowed)
    return orig_fdopendir(fd);
  errno = EACCES;
  return NULL;
}

// Interceptor for connect(). The TCP egress choke point: mirror the open()
// interceptor's re-entrancy guard so socket connects made by our own RPC
// client (which runs with in_sandbox==1) pass straight through. AF_UNIX and
// non-INET families are never mediated (sandbox_check_connect returns true for
// them). An out-of-policy refusal is a clean ECONNREFUSED, not an exit.
int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen) {
  ensure_init();
  if (in_sandbox)
    return orig_connect(sockfd, addr, addrlen);
  in_sandbox = 1;
  bool allowed = sandbox_check_connect(addr, addrlen);
  in_sandbox = 0;
  if (allowed)
    return orig_connect(sockfd, addr, addrlen);
  errno = ECONNREFUSED;
  return -1;
}

// Interceptor for getaddrinfo(). Resolution is never blocked — we only observe
// the result to attach hostnames to IPs for later connect() messages and
// hostname allow-net matching. The re-entrancy guard keeps our own internal
// resolutions (none today, but future-proof) from recursing.
int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints, struct addrinfo **res) {
  ensure_init();
  if (in_sandbox)
    return orig_getaddrinfo(node, service, hints, res);
  in_sandbox = 1;
  int rc = orig_getaddrinfo(node, service, hints, res);
  if (rc == 0 && res != NULL)
    net_observe_resolution(node, *res);
  in_sandbox = 0;
  return rc;
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
  in_dir_probe = (flags & O_DIRECTORY) ? 1 : 0;
  in_write_access = open_is_write(flags);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_dir_probe = 0;
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
  in_dir_probe = (flags & O_DIRECTORY) ? 1 : 0;
  in_write_access = open_is_write(flags);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_dir_probe = 0;
  in_sandbox = 0;
  if (allowed)
    return openat(dirfd, pathname, flags, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for fopen (macOS). Also the interposer for fopen$DARWIN_EXTSN
// (wired below), so it covers both the plain and extended-standards variants.
FILE *my_fopen(const char *pathname, const char *mode) {
  ensure_init();
  if (in_sandbox)
    return fopen(pathname, mode);
  in_sandbox = 1;
  in_write_access = fopen_is_write(mode);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return fopen(pathname, mode);
  errno = EACCES;
  return NULL;
}

// Interceptor for creat (macOS) — open(path, O_WRONLY|O_CREAT|O_TRUNC) under
// another name, so always a write. We reach the real creat() by calling creat()
// (a self-call is not interposed).
int my_creat(const char *pathname, mode_t mode) {
  ensure_init();
  if (in_sandbox)
    return creat(pathname, mode);
  in_sandbox = 1;
  in_write_access = 1;
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return creat(pathname, mode);
  errno = EACCES;
  return -1;
}

// Interceptor for truncate (macOS) — a path-addressed destructive write.
// ftruncate(fd, len) has no path and cannot be path-mediated (documented
// residual limit; the fd had to pass an open() check first).
int my_truncate(const char *path, off_t length) {
  ensure_init();
  if (in_sandbox)
    return truncate(path, length);
  in_sandbox = 1;
  in_write_access = 1;
  bool allowed = sandbox_check_path(path);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return truncate(path, length);
  errno = EACCES;
  return -1;
}

// Interceptor for freopen (macOS). Also the interposer for freopen$DARWIN_EXTSN
// (wired below), mirroring my_fopen. A NULL pathname reopens the existing
// stream in place (mode change only) — no path to mediate — so it is passed
// through.
FILE *my_freopen(const char *pathname, const char *mode, FILE *stream) {
  ensure_init();
  if (in_sandbox || pathname == NULL)
    return freopen(pathname, mode, stream);
  in_sandbox = 1;
  in_write_access = fopen_is_write(mode);
  bool allowed = sandbox_check_path(pathname);
  in_write_access = 0;
  in_sandbox = 0;
  if (allowed)
    return freopen(pathname, mode, stream);
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

// Interceptor for readlink (the non-at POSIX form) on macOS, for symmetry with
// the Linux interceptor: also advisory (warned-but-permitted). The __*_chk
// fortify variants are glibc-specific and have no macOS counterpart.
ssize_t my_readlink(const char *pathname, char *buf, size_t bufsiz) {
  ensure_init();
  if (in_sandbox)
    return readlink(pathname, buf, bufsiz);
  in_sandbox = 1;
  in_readlink = 1;
  bool allowed = sandbox_check_path(pathname);
  in_readlink = 0;
  in_sandbox = 0;
  if (allowed)
    return readlink(pathname, buf, bufsiz);
  errno = EACCES;
  return -1;
}

// Interceptor for opendir (macOS) — the directory-enumeration choke point.
// `ls` and shell globs list directories via opendir()/readdir(), which on
// this platform opens the directory through open$NOCANCEL (deliberately not
// interposed, see the note at the end of this file), so without this
// interceptor an enumeration reaches no check at all. We reach the real
// opendir() by calling opendir() (a self-call is not interposed) — which is
// also why is_directory()'s internal probe never recurses here.
DIR *my_opendir(const char *name) {
  ensure_init();
  if (in_sandbox)
    return opendir(name);
  in_sandbox = 1;
  in_dir_read = 1;
  bool allowed = sandbox_check_path(name);
  in_dir_read = 0;
  in_sandbox = 0;
  if (allowed)
    return opendir(name);
  errno = EACCES;
  return NULL;
}

// Interceptor for fdopendir (macOS) — enumeration over an fd from an earlier
// open(O_DIRECTORY). The fd is mapped back to its directory path with
// fcntl(F_GETPATH); on mapping failure the call FAILS OPEN, because the fd
// can only have come from an open()/openat() that already passed the
// O_DIRECTORY probe check.
DIR *my_fdopendir(int fd) {
  ensure_init();
  if (in_sandbox)
    return fdopendir(fd);
  in_sandbox = 1;
  bool allowed = true;
  char fd_path[PATH_MAX];
  if (fcntl(fd, F_GETPATH, fd_path) == 0) {
    in_dir_read = 1;
    allowed = sandbox_check_path(fd_path);
    in_dir_read = 0;
  }
  in_sandbox = 0;
  if (allowed)
    return fdopendir(fd);
  errno = EACCES;
  return NULL;
}

// Interceptor for connect() (macOS). Like the Linux one: the TCP egress choke
// point, refusing out-of-policy destinations with ECONNREFUSED rather than an
// exit. We reach the real connect() by calling connect() (a self-call is not
// interposed). AF_UNIX and non-INET destinations are never mediated.
int my_connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen) {
  ensure_init();
  if (in_sandbox)
    return connect(sockfd, addr, addrlen);
  in_sandbox = 1;
  bool allowed = sandbox_check_connect(addr, addrlen);
  in_sandbox = 0;
  if (allowed)
    return connect(sockfd, addr, addrlen);
  errno = ECONNREFUSED;
  return -1;
}

// Interceptor for getaddrinfo() (macOS). Observation only — resolution is
// never blocked; we record IP -> hostname for later connect() attribution.
int my_getaddrinfo(const char *node, const char *service,
                   const struct addrinfo *hints, struct addrinfo **res) {
  ensure_init();
  if (in_sandbox)
    return getaddrinfo(node, service, hints, res);
  in_sandbox = 1;
  int rc = getaddrinfo(node, service, hints, res);
  if (rc == 0 && res != NULL)
    net_observe_resolution(node, *res);
  in_sandbox = 0;
  return rc;
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
DYLD_INTERPOSE(my_creat, creat)
DYLD_INTERPOSE(my_truncate, truncate)
DYLD_INTERPOSE(my_freopen, freopen)
DYLD_INTERPOSE(my_readlinkat, readlinkat)
DYLD_INTERPOSE(my_readlink, readlink)
DYLD_INTERPOSE(my_opendir, opendir)
DYLD_INTERPOSE(my_fdopendir, fdopendir)
DYLD_INTERPOSE(my_connect, connect)
DYLD_INTERPOSE(my_getaddrinfo, getaddrinfo)

// On x86_64 macOS, <dirent.h> rewrites the `opendir`/`fdopendir` identifiers
// to their $INODE64 variants (the 64-bit-inode ABI, default since 10.6), so
// the interpose entries above already cover the symbols modern binaries bind
// there. Cover the legacy plain symbols too, via asm-aliased declarations,
// so a 32-bit-inode binary cannot enumerate unmediated — the same dual-
// binding pattern as fopen$DARWIN_EXTSN below. arm64 has no $INODE64
// variants (64-bit inodes are the only ABI), so the identifiers above ARE
// the plain symbols and these aliases must not be declared.
#if defined(__x86_64__)
extern DIR *opendir_plain(const char *name) __asm__("_opendir");
DYLD_INTERPOSE(my_opendir, opendir_plain)
extern DIR *fdopendir_plain(int fd) __asm__("_fdopendir");
DYLD_INTERPOSE(my_fdopendir, fdopendir_plain)
#endif

// macOS exports a second fopen, fopen$DARWIN_EXTSN (the "extended standards"
// variant). Binaries built in Darwin C mode — including the Nix coreutils
// `sum`/`cksum` — bind to it rather than plain fopen, so interposing only
// `fopen` lets those file reads slip past the sandbox. Interpose the variant
// too, via an asm-aliased declaration of its real symbol name.
extern FILE *
fopen_darwin_extsn(const char *pathname,
                   const char *mode) __asm__("_fopen$DARWIN_EXTSN");
DYLD_INTERPOSE(my_fopen, fopen_darwin_extsn)

// NOTE: macOS exports $NOCANCEL variants of the open-family syscalls
// (open$NOCANCEL / openat$NOCANCEL). They are deliberately NOT interposed:
// dyld uses them internally to map dylibs, so routing them through the policy
// check (which calls realpath()/opendir()/fopen() and can trigger lazy dylib
// binding) re-enters dyld from inside its own file I/O and the process is
// SIGKILLed. Application-level write-opens on this platform bind the plain
// `open`/`openat` symbols (verified: the shell `>>` redirect is mediated via
// the plain interceptors), so coverage does not depend on the variants — the
// real-tool `>>` regression test in the suite guards this end to end.
//
// Likewise, unlike fopen, freopen has no $DARWIN_EXTSN variant exported by
// libc on this platform, so only the plain `freopen` symbol is interposed.

#endif
