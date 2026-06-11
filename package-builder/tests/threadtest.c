/*
 * threadtest.c — thread-safety regression test for the Flox "virtual sandbox".
 *
 * WHY THIS EXISTS
 * ---------------
 * The sandbox shared library (libsandbox.so on Linux, libsandbox.dylib on
 * macOS) is loaded into *every* process spawned during a manifest build via
 * LD_PRELOAD / DYLD_INSERT_LIBRARIES. Real build tools are heavily threaded
 * (linkers, compilers, `make -j`, language runtimes), so the interception
 * code runs concurrently from many threads at once. Several pieces of state
 * in closure.c / sandbox.c are held in *shared, mutable, file-scope* buffers
 * (e.g. `realpath_buf`, `pkgbuf`, `real_path`) and guarded by a mutex that is
 * never initialized. On Linux a zero-initialized mutex happens to be a valid
 * `PTHREAD_MUTEX_INITIALIZER`, so the bug stays hidden; on macOS the same
 * mutex is invalid and locking silently becomes a no-op. Either way the shared
 * buffers are a data race waiting to corrupt an answer.
 *
 * HOW IT DETECTS THE BUG WITHOUT A SANITIZER
 * ------------------------------------------
 * ThreadSanitizer would be the obvious tool, but its runtime does not work
 * with the Nix-provided clang on this machine. Instead we use a *behavioral
 * oracle*: we call `in_closure()` — whose return value is fully determined by
 * its argument — from many threads, where each thread always passes the *same*
 * path and therefore always expects the *same* answer:
 *
 *   - "IN" threads check a path that lives inside the recorded closure and so
 *     must always return `true`.
 *   - "OUT" threads check a path that is *not* in the closure and so must
 *     always return `false`.
 *
 * If the implementation were thread-safe, no thread could ever observe an
 * answer meant for a different path. But because `realpath()` writes into a
 * shared `realpath_buf` and the hash lookup reads from a shared `pkgbuf`, an
 * IN thread can have its buffer clobbered by a concurrent OUT thread (and vice
 * versa) in the window between "write the resolved path" and "look it up".
 * When that happens the thread observes the *wrong* answer, which we count as
 * a mismatch. Any mismatch at all is a positive reproduction of the race.
 *
 * This makes the failure deterministic in spirit (it reliably reproduces with
 * enough threads/iterations) and, unlike a sanitizer report, it runs anywhere.
 *
 * EXIT STATUS
 * -----------
 *   0  -> no mismatches observed: the code under test behaved as if race-free.
 *   1  -> at least one mismatch: a data race corrupted an answer (bug present).
 *   2  -> test could not be set up (e.g. could not find two usable store
 *         paths, or could not write the fixture). This is a test-harness
 *         problem, not a verdict about the sandbox.
 */

#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <limits.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

/* in_closure() is the function under test; it is the shared core of both the
 * Linux and macOS sandbox libraries, so exercising it here covers both. */
#include "../closure.h"

/* Tunables. The defaults are deliberately aggressive: realpath() does a lstat
 * per path component, which keeps the race window wide, and tens of thousands
 * of iterations across many threads makes an existing race overwhelmingly
 * likely to be observed at least once. Both can be overridden from the
 * environment so the same binary can be used for a quick smoke test or a long
 * soak test. */
#define DEFAULT_NTHREADS 16
#define DEFAULT_NITERS 20000

/* The /nix/store directory entries we sample to build the test fixture. A
 * valid store path name is a 32-character base-32 hash, a dash, then a
 * human-readable name, so the shortest possible name is comfortably longer
 * than the 44-character prefix the closure hash table keys on. We require
 * names at least this long to stay clear of the (separately tracked)
 * out-of-bounds read for sub-44-character paths in hash_table_lookup(). */
#define MIN_STORE_NAME_LEN 45
#define NIX_STORE_DIR "/nix/store"

/* Shared, read-only-after-setup probe paths. They are assigned once in main()
 * before any worker thread starts, so reading them concurrently is safe.
 *
 * The closure hash table keys on the store-object prefix and, crucially,
 * hash_table_lookup() only matches a key that has a path component *after* the
 * store object (it locates the first '/' past the 44-character hash prefix).
 * Real build-time accesses always look like `<store-object>/bin/foo`, so each
 * probe must likewise point *inside* a store directory, not at the bare store
 * path. We therefore record both the store-object root (what goes into
 * requisites.txt) and a concrete child path inside it (what the threads
 * probe). */
static char
    requisite_in[PATH_MAX];    /* store-object root listed in requisites.txt */
static char path_in[PATH_MAX]; /* child inside requisite_in -> expect true    */
static char path_out[PATH_MAX]; /* child inside an *unlisted* store object */

/* Outcome counters. Updated from every worker thread, hence atomic. */
static atomic_long total_checks = 0; /* how many in_closure() calls we made   */
static atomic_long mismatches = 0; /* how many returned the wrong answer     */

/* Worker that repeatedly asserts an in-closure path is reported as in-closure.
 * A `false` result here can only happen if another thread corrupted the shared
 * resolution buffers mid-call, so we count it as a mismatch. */
static void *worker_in(void *arg) {
  long iters = *(long *)arg;
  for (long i = 0; i < iters; i++) {
    bool result = in_closure(path_in);
    atomic_fetch_add_explicit(&total_checks, 1, memory_order_relaxed);
    if (!result) {
      atomic_fetch_add_explicit(&mismatches, 1, memory_order_relaxed);
    }
  }
  return NULL;
}

/* Mirror of worker_in for a path that must never be considered in-closure. */
static void *worker_out(void *arg) {
  long iters = *(long *)arg;
  for (long i = 0; i < iters; i++) {
    bool result = in_closure(path_out);
    atomic_fetch_add_explicit(&total_checks, 1, memory_order_relaxed);
    if (result) {
      atomic_fetch_add_explicit(&mismatches, 1, memory_order_relaxed);
    }
  }
  return NULL;
}

/* Read a positive long from the environment, falling back to `dflt` when the
 * variable is unset or not parseable. Keeps the tunables in one place. */
static long env_long(const char *name, long dflt) {
  const char *value = getenv(name);
  if (value == NULL || *value == '\0') {
    return dflt;
  }
  char *end = NULL;
  long parsed = strtol(value, &end, 10);
  if (end == value || parsed <= 0) {
    return dflt;
  }
  return parsed;
}

/*
 * Given a candidate store-object directory, find any concrete child entry
 * inside it and write the resolved "<root>/<child>" path into `probe_out`
 * (size PATH_MAX). Returns 0 on success, -1 if the candidate is not a usable
 * directory or has no children we can resolve.
 *
 * Resolving through realpath() matters: it both proves the path exists (so the
 * answer is unambiguous — in_closure() treats non-existent paths as allowed)
 * and matches the value in_closure() computes internally.
 */
static int resolve_child_in(const char *root, char *probe_out) {
  DIR *dir = opendir(root);
  if (dir == NULL) {
    return -1; /* not a directory, or unreadable */
  }
  int ok = -1;
  struct dirent *entry;
  while ((entry = readdir(dir)) != NULL) {
    if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
      continue;
    }
    char child[PATH_MAX];
    int written = snprintf(child, sizeof(child), "%s/%s", root, entry->d_name);
    if (written < 0 || (size_t)written >= sizeof(child)) {
      continue;
    }
    if (realpath(child, probe_out) != NULL) {
      ok = 0;
      break;
    }
  }
  closedir(dir);
  return ok;
}

/*
 * Populate the three probe paths from two *distinct* /nix/store directories:
 *   requisite_in -> the store-object root that we will list in requisites.txt
 *   path_in      -> a child inside requisite_in (must be reported in-closure)
 *   path_out     -> a child inside a *different*, unlisted store object
 * Returns 0 on success, -1 if two usable store directories could not be found.
 */
static int pick_probe_paths(void) {
  DIR *dir = opendir(NIX_STORE_DIR);
  if (dir == NULL) {
    fprintf(stderr, "threadtest: cannot open %s: %s\n", NIX_STORE_DIR,
            strerror(errno));
    return -1;
  }

  bool have_in = false;
  bool have_out = false;
  struct dirent *entry;
  while (!(have_in && have_out) && (entry = readdir(dir)) != NULL) {
    /* Skip "." / ".." and names too short to be well-formed store paths (also
     * keeps every probe comfortably longer than the 44-char hash prefix). */
    if (strlen(entry->d_name) < MIN_STORE_NAME_LEN) {
      continue;
    }

    char root[PATH_MAX];
    int written =
        snprintf(root, sizeof(root), "%s/%s", NIX_STORE_DIR, entry->d_name);
    if (written < 0 || (size_t)written >= sizeof(root)) {
      continue;
    }

    /* It must be a directory with at least one resolvable child, otherwise we
     * cannot build a probe that exercises the "<store>/<component>" lookup. */
    char probe[PATH_MAX];
    if (resolve_child_in(root, probe) != 0) {
      continue;
    }

    if (!have_in) {
      /* First usable directory becomes the in-closure object. */
      snprintf(requisite_in, sizeof(requisite_in), "%s", root);
      snprintf(path_in, sizeof(path_in), "%s", probe);
      have_in = true;
    } else {
      /* Second, distinct directory becomes the out-of-closure object. */
      snprintf(path_out, sizeof(path_out), "%s", probe);
      have_out = true;
    }
  }
  closedir(dir);

  if (!(have_in && have_out)) {
    fprintf(
        stderr,
        "threadtest: could not find two usable store directories under %s\n",
        NIX_STORE_DIR);
    return -1;
  }
  return 0;
}

/* Path of the temp fixture directory to remove at exit. Set by write_fixture()
 * once the directory exists, so the fixture is cleaned up on every exit path
 * (success, the early "setup error" returns, and normal completion). */
static char fixture_to_clean[PATH_MAX];

/* atexit handler: remove the fixture (only ever contains requisites.txt). */
static void cleanup_fixture(void) {
  if (fixture_to_clean[0] == '\0') {
    return;
  }
  char requisites_path[PATH_MAX];
  if (snprintf(requisites_path, sizeof(requisites_path), "%s/requisites.txt",
               fixture_to_clean) > 0) {
    unlink(requisites_path);
  }
  rmdir(fixture_to_clean);
}

/*
 * Build the minimal $FLOX_ENV fixture in-place: a freshly created temp
 * directory containing a requisites.txt that lists ONLY path_in. That makes
 * path_in "in the closure" and path_out (deliberately omitted) "out of it".
 * On success, points FLOX_ENV at the fixture and returns 0.
 */
static int write_fixture(char *env_dir, size_t env_dir_size) {
  /* mkdtemp wants a writable template buffer ending in XXXXXX. */
  if (snprintf(env_dir, env_dir_size, "/tmp/flox-sandbox-threadtest-XXXXXX") <
      0) {
    return -1;
  }
  if (mkdtemp(env_dir) == NULL) {
    fprintf(stderr, "threadtest: mkdtemp failed: %s\n", strerror(errno));
    return -1;
  }
  /* Register cleanup now that the directory exists, so it is removed even if a
   * subsequent setup step fails and main() returns early. */
  snprintf(fixture_to_clean, sizeof(fixture_to_clean), "%s", env_dir);
  atexit(cleanup_fixture);

  char requisites_path[PATH_MAX];
  if (snprintf(requisites_path, sizeof(requisites_path), "%s/requisites.txt",
               env_dir) < 0) {
    return -1;
  }

  FILE *file = fopen(requisites_path, "w");
  if (file == NULL) {
    fprintf(stderr, "threadtest: cannot write %s: %s\n", requisites_path,
            strerror(errno));
    return -1;
  }
  /* One requisite: the store-object root of the in-closure probe. (Listing the
   * root mirrors how Flox records closures — requisites.txt holds store-object
   * paths, and lookups for files beneath them match on the shared hash.) */
  fprintf(file, "%s\n", requisite_in);
  fclose(file);

  /* in_closure() reads FLOX_ENV exactly once (it caches the parsed table), so
   * this must be set before the first call below. */
  if (setenv("FLOX_ENV", env_dir, 1) != 0) {
    return -1;
  }
  return 0;
}

int main(void) {
  long nthreads = env_long("THREADTEST_NTHREADS", DEFAULT_NTHREADS);
  long niters = env_long("THREADTEST_NITERS", DEFAULT_NITERS);

  /* Stage 1: choose two real store paths and record which is in the closure. */
  if (pick_probe_paths() != 0) {
    return 2;
  }
  char env_dir[PATH_MAX];
  if (write_fixture(env_dir, sizeof(env_dir)) != 0) {
    return 2;
  }

  fprintf(stderr, "threadtest: FLOX_ENV=%s\n", env_dir);
  fprintf(stderr, "threadtest: in-closure  probe = %s\n", path_in);
  fprintf(stderr, "threadtest: out-closure probe = %s\n", path_out);
  fprintf(stderr, "threadtest: %ld threads x %ld iters\n", nthreads, niters);

  /* Pre-warm the closure table on the main thread. This forces the one-time
   * parse of requisites.txt to complete *before* any concurrency, so that the
   * mismatches we measure come purely from the steady-state shared-buffer race
   * and not from a separate (also-real) table-initialization race. Both are
   * fixed by the thread-safety work; isolating them keeps this oracle crisp.
   *
   * It also doubles as a sanity check: if the fixture is wrong, the expected
   * answers won't hold even single-threaded, and we can bail with a clear
   * message instead of a confusing flood of "mismatches". */
  if (!in_closure(path_in)) {
    fprintf(stderr,
            "threadtest: setup error: in-closure probe not reported in closure"
            " (single-threaded). Fixture is wrong, aborting.\n");
    return 2;
  }
  if (in_closure(path_out)) {
    fprintf(stderr,
            "threadtest: setup error: out-closure probe reported in closure"
            " (single-threaded). Fixture is wrong, aborting.\n");
    return 2;
  }

  /* Stage 2: launch the storm. Half the threads assert IN, half assert OUT, so
   * that there is always cross-traffic competing for the shared buffers. */
  pthread_t *threads = calloc((size_t)nthreads, sizeof(pthread_t));
  if (threads == NULL) {
    fprintf(stderr, "threadtest: out of memory\n");
    return 2;
  }
  for (long i = 0; i < nthreads; i++) {
    void *(*fn)(void *) = (i % 2 == 0) ? worker_in : worker_out;
    if (pthread_create(&threads[i], NULL, fn, &niters) != 0) {
      fprintf(stderr, "threadtest: pthread_create failed at %ld\n", i);
      return 2;
    }
  }
  for (long i = 0; i < nthreads; i++) {
    pthread_join(threads[i], NULL);
  }
  free(threads);

  /* Stage 3: verdict. */
  long checks = atomic_load(&total_checks);
  long bad = atomic_load(&mismatches);
  fprintf(stderr, "threadtest: %ld checks, %ld mismatches\n", checks, bad);
  if (bad != 0) {
    fprintf(stderr,
            "threadtest: FAIL — %ld/%ld in_closure() results were corrupted by"
            " a concurrent call (data race reproduced).\n",
            bad, checks);
    return 1;
  }
  fprintf(stderr, "threadtest: PASS — all results were correct under"
                  " concurrency.\n");
  return 0;
}
