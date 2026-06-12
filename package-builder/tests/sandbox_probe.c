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
 *       accordingly. (In "enforce" mode an out-of-policy open is refused with
 *       EACCES — a clean failure the probe reports, never a process abort.)
 *
 *   sandbox_probe storm <nthreads> <niters> <path1> [path2 ...]
 *       Spawn <nthreads> threads, each opening the given paths in round-robin
 *       <niters> times, immediately closing any successful descriptor. This is
 *       the threaded stress test: success is simply "ran to completion without
 *       crashing or hanging" (exit 0). Intended for "off"/"warn" modes, where
 *       no open is fatal.
 */

#include <arpa/inet.h>
#include <dirent.h>
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
#include <time.h>
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

/* open-twice — open <path>, sleep <secs>, open <path> again, all in ONE
 * process so the sandbox library's per-process decision cache persists across
 * the two opens. This exercises the prompt deny-cache TTL: a first deny is
 * cached for ~2s; sleeping past the TTL and re-opening forces a fresh broker
 * RPC, so a broker that flipped deny->allow during the sleep is observed on the
 * second open. Prints "FIRST <OPEN_OK|OPEN_FAIL>" then "SECOND
 * <OPEN_OK|OPEN_FAIL>". Exits 0 iff the second open succeeded (the TTL-expiry
 * case under test). */
static int do_open_twice(const char *path, double secs) {
  int fd1 = open(path, O_RDONLY);
  if (fd1 >= 0) {
    close(fd1);
    printf("FIRST OPEN_OK %s\n", path);
  } else {
    printf("FIRST OPEN_FAIL %s errno=%d\n", path, errno);
  }
  struct timespec ts;
  ts.tv_sec = (time_t)secs;
  ts.tv_nsec = (long)((secs - (double)ts.tv_sec) * 1e9);
  nanosleep(&ts, NULL);
  int fd2 = open(path, O_RDONLY);
  if (fd2 >= 0) {
    close(fd2);
    printf("SECOND OPEN_OK %s\n", path);
    return 0;
  }
  printf("SECOND OPEN_FAIL %s errno=%d\n", path, errno);
  return 1;
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

/* write — open an EXISTING <path> for writing (O_WRONLY, no O_CREAT) so it has
 * a realpath and is classified as a write access. Unlike `create` (a new file
 * judged by its parent), this exercises the grants-dir write guard, which fires
 * on a write to an existing path under FLOX_SANDBOX_GRANTS_DIR. Prints
 * "WRITE_OK <path>" on success or "WRITE_FAIL <path> errno=<n> (<msg>)" on
 * refusal, and exits 0/1. */
static int do_write(const char *path) {
  int fd = open(path, O_WRONLY);
  if (fd < 0) {
    int saved = errno;
    printf("WRITE_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  close(fd);
  printf("WRITE_OK %s\n", path);
  return 0;
}

/* append — open an EXISTING <path> with O_WRONLY|O_APPEND, the access a shell
 * `>>` redirect performs. The file has a realpath, so it is classified as a
 * write of an existing path and (under an activation) reaches the sensitive-set
 * check the same way an existing-file read does. Prints "APPEND_OK <path>" on
 * success or "APPEND_FAIL <path> errno=<n> (<msg>)" on refusal, and exits 0/1.
 * The file is opened but never written to, so a permitted append leaves it
 * unchanged. */
static int do_append(const char *path) {
  int fd = open(path, O_WRONLY | O_APPEND);
  if (fd < 0) {
    int saved = errno;
    printf("APPEND_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  close(fd);
  printf("APPEND_OK %s\n", path);
  return 0;
}

/* creat — create <path> via creat(), a distinct libc entry point from
 * open(O_CREAT). The target is expected NOT to exist, so this is a genuine
 * new-file create judged by its parent directory's policy under an activation.
 * Exercises the creat() interceptor specifically (a tool binding creat rather
 * than open must not slip past the sandbox). Prints "CREAT_OK"/"CREAT_FAIL" and
 * exits 0/1; any file created is unlinked so reruns stay idempotent. */
static int do_creat(const char *path) {
  int fd = creat(path, 0644);
  if (fd < 0) {
    int saved = errno;
    printf("CREAT_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  close(fd);
  unlink(path);
  printf("CREAT_OK %s\n", path);
  return 0;
}

/* truncate — truncate an EXISTING <path> to zero length via truncate(). A
 * truncate is a destructive write that takes a path (not an fd), so it must be
 * mediated like an open-for-write. Prints "TRUNCATE_OK"/"TRUNCATE_FAIL" and
 * exits 0/1. */
static int do_truncate(const char *path) {
  if (truncate(path, 0) != 0) {
    int saved = errno;
    printf("TRUNCATE_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  printf("TRUNCATE_OK %s\n", path);
  return 0;
}

/* freopen — reopen a throwaway stream onto <path> for writing. freopen() is a
 * libc entry point distinct from fopen()/open() that opens (and truncates) a
 * file; a tool binding it must not slip past the sandbox. We first fopen()
 * /dev/null (an allow-dir, so permitted) to get a base stream, then freopen()
 * it onto <path> — the call under test. Prints "FREOPEN_OK"/"FREOPEN_FAIL" and
 * exits 0/1. */
static int do_freopen(const char *path) {
  FILE *base = fopen("/dev/null", "w");
  if (base == NULL) {
    printf("FREOPEN_SETUP_FAIL errno=%d (%s)\n", errno, strerror(errno));
    return 2;
  }
  FILE *f = freopen(path, "w", base);
  if (f == NULL) {
    int saved = errno;
    printf("FREOPEN_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    fclose(base);
    return 1;
  }
  printf("FREOPEN_OK %s\n", path);
  fclose(f);
  return 0;
}

/* survive — prove a denied write does NOT terminate a long-lived process. A
 * policy denial must surface as a graceful errno (EACCES), never an exit(): a
 * shell builtin redirect performs its open inside the interactive shell
 * process itself, so a fatal denial would kill the user's shell. This opens
 * <denied> for append (expected to be refused), then — STILL RUNNING — opens
 * <allowed> for read (expected to succeed), and prints a final "SURVIVED"
 * line. Always returns 0: a process killed at the denied write never reaches
 * the SURVIVED print, so the test discriminates on the output and exit status
 * together. */
static int do_survive(const char *denied, const char *allowed) {
  int fd1 = open(denied, O_WRONLY | O_APPEND);
  if (fd1 < 0) {
    printf("APPEND_DENIED %s errno=%d\n", denied, errno);
  } else {
    close(fd1);
    printf("APPEND_OK %s\n", denied);
  }
  int fd2 = open(allowed, O_RDONLY);
  if (fd2 < 0) {
    printf("READ_FAIL %s errno=%d\n", allowed, errno);
  } else {
    close(fd2);
    printf("READ_OK %s\n", allowed);
  }
  printf("SURVIVED\n");
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

/* opendir — enumerate <path> via opendir()+readdir(), the entry points ls and
 * shell globs bind. Under an activation an out-of-policy enumeration is
 * mediated as a READ of the directory path: warn reports and permits,
 * enforce/prompt refuse with a graceful EACCES from opendir() (never a process
 * abort). Prints "OPENDIR_OK <path> entries=<n>" on success or
 * "OPENDIR_FAIL <path> errno=<n> (<msg>)" on refusal, and exits 0/1. */
static int do_opendir(const char *path) {
  DIR *dir = opendir(path);
  if (dir == NULL) {
    int saved = errno;
    printf("OPENDIR_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    return 1;
  }
  int entries = 0;
  while (readdir(dir) != NULL)
    entries++;
  closedir(dir);
  printf("OPENDIR_OK %s entries=%d\n", path, entries);
  return 0;
}

/* fdopendir — open <path> with O_DIRECTORY, then enumerate via fdopendir()
 * (the openat()+fdopendir() traversal style used by find and fts). The
 * open(O_DIRECTORY) is a warned-but-permitted probe, so the fd is obtained
 * even out of policy; fdopendir() maps the fd back to its directory path and
 * applies the directory-read verdict before any entry is readable. Prints
 * "FDOPENDIR_OK <path> entries=<n>" or "FDOPENDIR_FAIL <path> errno=<n>
 * (<msg>)" (or FDOPENDIR_SETUP_FAIL if the open itself failed) and exits
 * 0/1/2. */
static int do_fdopendir(const char *path) {
  int fd = open(path, O_RDONLY | O_NONBLOCK | O_DIRECTORY);
  if (fd < 0) {
    printf("FDOPENDIR_SETUP_FAIL %s errno=%d (%s)\n", path, errno,
           strerror(errno));
    return 2;
  }
  DIR *dir = fdopendir(fd);
  if (dir == NULL) {
    int saved = errno;
    printf("FDOPENDIR_FAIL %s errno=%d (%s)\n", path, saved, strerror(saved));
    close(fd);
    return 1;
  }
  int entries = 0;
  while (readdir(dir) != NULL)
    entries++;
  closedir(dir); /* also closes fd */
  printf("FDOPENDIR_OK %s entries=%d\n", path, entries);
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
  if (argc >= 4 && strcmp(argv[1], "open-twice") == 0) {
    return do_open_twice(argv[2], strtod(argv[3], NULL));
  }
  if (argc >= 3 && strcmp(argv[1], "create") == 0) {
    return do_create(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "write") == 0) {
    return do_write(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "append") == 0) {
    return do_append(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "creat") == 0) {
    return do_creat(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "truncate") == 0) {
    return do_truncate(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "freopen") == 0) {
    return do_freopen(argv[2]);
  }
  if (argc >= 4 && strcmp(argv[1], "survive") == 0) {
    return do_survive(argv[2], argv[3]);
  }
  if (argc >= 3 && strcmp(argv[1], "open-dir") == 0) {
    return do_open_dir(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "opendir") == 0) {
    return do_opendir(argv[2]);
  }
  if (argc >= 3 && strcmp(argv[1], "fdopendir") == 0) {
    return do_fdopendir(argv[2]);
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
          "  %s open-twice <path> <sleep_secs>\n"
          "  %s create <path>\n"
          "  %s write <path>\n"
          "  %s append <path>\n"
          "  %s creat <path>\n"
          "  %s truncate <path>\n"
          "  %s freopen <path>\n"
          "  %s survive <denied-path> <allowed-path>\n"
          "  %s open-dir <path>\n"
          "  %s opendir <path>\n"
          "  %s fdopendir <path>\n"
          "  %s readlink <path>\n"
          "  %s readlink-fn <path>\n"
          "  %s connect <ipv4> <port> [timeout_ms]\n"
          "  %s storm <nthreads> <niters> <path1> [path2 ...]\n",
          argv[0], argv[0], argv[0], argv[0], argv[0], argv[0], argv[0],
          argv[0], argv[0], argv[0], argv[0], argv[0], argv[0], argv[0],
          argv[0], argv[0]);
  return 2;
}
