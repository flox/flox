/*
 * sandbox_probe.c — a tiny "build process" stand-in used to exercise the
 * sandbox library through the *real* loader-interposition path.
 *
 * Unlike threadtest.c (which links the policy functions directly and calls
 * in_closure() in-process), this program is launched as a separate process
 * with the sandbox library injected via DYLD_INSERT_LIBRARIES (macOS) or
 * LD_PRELOAD (Linux). That means its open()/openat() calls actually flow
 * through the interceptors, so it validates the parts the in-process oracle
 * cannot: the macOS DYLD_INTERPOSE wiring, the per-thread re-entrancy guard,
 * and the one-time initialization happening under genuine concurrency.
 *
 * Usage:
 *   sandbox_probe open  <path>
 *       Open <path> once. Prints "OPEN_OK <path>" on success or
 *       "OPEN_FAIL <path> errno=<n> (<msg>)" on failure, and exits 0/1
 *       accordingly. (In "enforce" mode the sandbox terminates the process
 *       itself before we can print, which is exactly the behavior under test.)
 *
 *   sandbox_probe storm <nthreads> <niters> <path1> [path2 ...]
 *       Spawn <nthreads> threads, each opening the given paths in round-robin
 *       <niters> times, immediately closing any successful descriptor. This is
 *       the threaded stress test: success is simply "ran to completion without
 *       crashing or hanging" (exit 0). Intended for "off"/"warn" modes, where
 *       no open is fatal.
 */

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Arguments handed to each storm worker. */
typedef struct {
  char **paths; /* NULL-terminated list of paths to cycle through */
  int npaths;
  long niters;
} storm_args_t;

/* Worker body: open each path in turn, niters times, closing successes. We do
 * not assert on the result because in "warn"/"off" mode every open is allowed
 * and in-closure vs out-of-closure is not observable here; the point is to
 * drive the interceptors hard from many threads at once. */
static void *storm_worker(void *arg) {
  storm_args_t *a = (storm_args_t *)arg;
  for (long i = 0; i < a->niters; i++) {
    const char *path = a->paths[i % a->npaths];
    int fd = open(path, O_RDONLY);
    if (fd >= 0) {
      close(fd);
    }
  }
  return NULL;
}

static int do_open(const char *path) {
  int fd = open(path, O_RDONLY);
  if (fd < 0) {
    /* Capture errno before any other call can clobber it. */
    int saved = errno;
    printf("OPEN_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  close(fd);
  printf("OPEN_OK %s\n", path);
  return 0;
}

/* open(O_DIRECTORY) — a directory probe that cannot read file contents.
 * The kernel returns ENOTDIR for non-directory paths regardless; this
 * exercises the in_dir_probe path in the sandbox interceptors. */
static int do_open_dir(const char *path) {
  int fd = open(path, O_RDONLY | O_NONBLOCK | O_DIRECTORY);
  if (fd < 0) {
    int saved = errno;
    printf("OPEN_DIR_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    /* ENOTDIR / ENOENT are expected for non-directory paths — not a probe
     * failure, just the kernel doing the right thing. */
    return saved == ENOTDIR || saved == ENOENT ? 0 : 1;
  }
  close(fd);
  printf("OPEN_DIR_OK %s\n", path);
  return 0;
}

/* readlinkat(AT_FDCWD, ...) so the readlinkat interceptor is exercised. */
static int do_readlink(const char *path) {
  char buf[PATH_MAX];
  ssize_t n = readlinkat(AT_FDCWD, path, buf, sizeof(buf) - 1);
  if (n < 0) {
    int saved = errno;
    printf("READLINK_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  buf[n] = '\0';
  printf("READLINK_OK %s -> %s\n", path, buf);
  return 0;
}

/* readlink() (the non-at form) so the readlink interceptor is exercised.
 * This is compiled without _FORTIFY_SOURCE, so it calls the plain readlink
 * symbol rather than __readlink_chk. */
static int do_readlink_fn(const char *path) {
  char buf[PATH_MAX];
  ssize_t n = readlink(path, buf, sizeof(buf) - 1);
  if (n < 0) {
    int saved = errno;
    printf("READLINK_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  buf[n] = '\0';
  printf("READLINK_OK %s -> %s\n", path, buf);
  return 0;
}

static int do_storm(int argc, char **argv) {
  /* argv: [0]=prog [1]="storm" [2]=nthreads [3]=niters [4..]=paths */
  long nthreads = strtol(argv[2], NULL, 10);
  long niters = strtol(argv[3], NULL, 10);
  if (nthreads <= 0 || niters <= 0 || argc < 5) {
    fprintf(stderr, "sandbox_probe: bad storm arguments\n");
    return 2;
  }

  storm_args_t args = {
      .paths = &argv[4],
      .npaths = argc - 4,
      .niters = niters,
  };

  pthread_t *threads = calloc((size_t)nthreads, sizeof(pthread_t));
  if (threads == NULL) {
    fprintf(stderr, "sandbox_probe: out of memory\n");
    return 2;
  }
  for (long i = 0; i < nthreads; i++) {
    if (pthread_create(&threads[i], NULL, storm_worker, &args) != 0) {
      fprintf(stderr, "sandbox_probe: pthread_create failed at %ld\n", i);
      return 2;
    }
  }
  for (long i = 0; i < nthreads; i++) {
    pthread_join(threads[i], NULL);
  }
  free(threads);

  printf("STORM_OK %ld threads x %ld iters over %d paths\n", nthreads, niters,
         args.npaths);
  return 0;
}

int main(int argc, char **argv) {
  if (argc >= 3 && strcmp(argv[1], "open") == 0) {
    return do_open(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "open-dir") == 0) {
    return do_open_dir(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "readlink") == 0) {
    return do_readlink(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "readlink-fn") == 0) {
    return do_readlink_fn(argv[2]);
  }
  if (argc >= 5 && strcmp(argv[1], "storm") == 0) {
    return do_storm(argc, argv);
  }
  fprintf(stderr,
          "usage:\n"
          "  %s open <path>\n"
          "  %s open-dir <path>\n"
          "  %s readlink <path>\n"
          "  %s readlink-fn <path>\n"
          "  %s storm <nthreads> <niters> <path1> [path2 ...]\n",
          argv[0], argv[0], argv[0], argv[0], argv[0]);
  return 2;
}
