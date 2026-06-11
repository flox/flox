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

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netinet/in.h>
#include <poll.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
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

/* create — open <path> for writing with O_CREAT, exercising the write-create
 * guard. The target is expected NOT to exist, so this is a genuine new-file
 * create: the sandbox judges it by its parent directory's policy under an
 * activation. Prints "CREATE_OK <path>" on success or
 * "CREATE_FAIL <path> errno=<n> (<msg>)" on refusal, and exits 0/1. Any file
 * actually created is unlinked so reruns stay idempotent. */
static int do_create(const char *path) {
  int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
  if (fd < 0) {
    int saved = errno;
    printf("CREATE_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  close(fd);
  unlink(path);
  printf("CREATE_OK %s\n", path);
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

/* connect() to <ipv4>:<port> with a SHORT timeout so a permitted-but-
 * unroutable destination (e.g. 192.0.2.1, TEST-NET-1) cannot hang the test.
 *
 * The socket is non-blocking so the interceptor's verdict is observed
 * synchronously:
 *   - sandbox refuses  -> connect() returns -1 with ECONNREFUSED immediately.
 *     Prints "CONNECT_REFUSED <ip>:<port> errno=<n>" and exits 1.
 *   - sandbox permits  -> connect() either succeeds at once (CONNECT_OK), or
 *     returns -1/EINPROGRESS and we poll up to timeout_ms. A timeout means the
 *     sandbox did NOT block (the connect reached the network and stalled),
 *     which is the warn/allowed case for an unroutable address; we print
 *     "CONNECT_PROCEEDED" and exit 0 so callers can assert "not refused".
 *
 * Usage: sandbox_probe connect <ipv4> <port> [timeout_ms]
 */
static int do_connect(const char *ip, int port, int timeout_ms) {
  int fd = socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0) {
    int saved = errno;
    printf("CONNECT_SOCKETFAIL %s:%d errno=%d (%s)\n", ip, port, saved,
           strerror(saved));
    return 2;
  }
  /* Non-blocking so the interceptor verdict is immediate and a real network
   * connect cannot block past our timeout. */
  int flags = fcntl(fd, F_GETFL, 0);
  fcntl(fd, F_SETFL, flags | O_NONBLOCK);

  struct sockaddr_in sa;
  memset(&sa, 0, sizeof(sa));
  sa.sin_family = AF_INET;
  sa.sin_port = htons((unsigned short)port);
  if (inet_pton(AF_INET, ip, &sa.sin_addr) != 1) {
    printf("CONNECT_BADIP %s\n", ip);
    close(fd);
    return 2;
  }

  int rc = connect(fd, (struct sockaddr *)&sa, sizeof(sa));
  if (rc == 0) {
    printf("CONNECT_OK %s:%d\n", ip, port);
    close(fd);
    return 0;
  }
  int saved = errno;
  /* The sandbox refuses out-of-policy destinations with ECONNREFUSED before
   * the syscall — this arrives immediately, never via EINPROGRESS. */
  if (saved == ECONNREFUSED) {
    printf("CONNECT_REFUSED %s:%d errno=%d (%s)\n", ip, port, saved,
           strerror(saved));
    close(fd);
    return 1;
  }
  if (saved != EINPROGRESS) {
    /* Some other immediate failure (e.g. ENETUNREACH). Not a sandbox refusal;
     * report it so the caller can see the connect was not blocked. */
    printf("CONNECT_PROCEEDED %s:%d errno=%d (%s)\n", ip, port, saved,
           strerror(saved));
    close(fd);
    return 0;
  }
  /* EINPROGRESS: the sandbox permitted the connect and the network layer is
   * working on it. Poll briefly; a timeout means "permitted, just unroutable",
   * which for our purposes is success (the sandbox did not block). */
  struct pollfd pfd = {.fd = fd, .events = POLLOUT};
  int pr = poll(&pfd, 1, timeout_ms);
  if (pr == 0) {
    printf("CONNECT_PROCEEDED %s:%d (timeout; not blocked by sandbox)\n", ip,
           port);
    close(fd);
    return 0;
  }
  int err = 0;
  socklen_t errlen = sizeof(err);
  getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &errlen);
  if (err == 0) {
    printf("CONNECT_OK %s:%d\n", ip, port);
  } else {
    printf("CONNECT_PROCEEDED %s:%d errno=%d (%s)\n", ip, port, err,
           strerror(err));
  }
  close(fd);
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
  if (argc >= 3 && strcmp(argv[1], "create") == 0) {
    return do_create(argv[2]);
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
  if (argc >= 4 && strcmp(argv[1], "connect") == 0) {
    int port = (int)strtol(argv[3], NULL, 10);
    int timeout_ms = (argc >= 5) ? (int)strtol(argv[4], NULL, 10) : 300;
    return do_connect(argv[2], port, timeout_ms);
  }
  if (argc >= 5 && strcmp(argv[1], "storm") == 0) {
    return do_storm(argc, argv);
  }
  fprintf(stderr,
          "usage:\n"
          "  %s open <path>\n"
          "  %s create <path>\n"
          "  %s open-dir <path>\n"
          "  %s readlink <path>\n"
          "  %s readlink-fn <path>\n"
          "  %s connect <ipv4> <port> [timeout_ms]\n"
          "  %s storm <nthreads> <niters> <path1> [path2 ...]\n",
          argv[0], argv[0], argv[0], argv[0], argv[0], argv[0], argv[0]);
  return 2;
}
