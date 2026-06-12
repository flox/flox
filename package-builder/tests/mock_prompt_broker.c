/*
 * mock_prompt_broker.c — a minimal stand-in for the Phase 2 prompt broker,
 * used by run-tests.sh to exercise libsandbox's prompt client without the real
 * (interactive) flox broker.
 *
 * It listens on the AF_UNIX socket path given as argv[1] and replies with a
 * fixed decision (argv[2], one of "allow", "deny", or an "allow-glob <pattern>"
 * line) to every query, following the same one-request/one-reply-per-connection
 * newline-terminated protocol libsandbox speaks. It prints "READY" once the
 * socket is listening so the test can wait for it, then serves forever until
 * killed.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

int main(int argc, char **argv) {
  if (argc < 3) {
    fprintf(stderr, "usage: %s <socket-path> <reply>\n", argv[0]);
    return 2;
  }
  const char *path = argv[1];
  const char *reply = argv[2];

  unlink(path); /* a stale socket file would make bind() fail */

  int listener = socket(AF_UNIX, SOCK_STREAM, 0);
  if (listener < 0) {
    perror("socket");
    return 1;
  }
  struct sockaddr_un addr;
  memset(&addr, 0, sizeof(addr));
  addr.sun_family = AF_UNIX;
  strncpy(addr.sun_path, path, sizeof(addr.sun_path) - 1);
  if (bind(listener, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
    perror("bind");
    return 1;
  }
  if (listen(listener, 16) != 0) {
    perror("listen");
    return 1;
  }

  /* Tell the test we are ready to accept connections. */
  printf("READY\n");
  fflush(stdout);

  for (;;) {
    int conn = accept(listener, NULL, NULL);
    if (conn < 0) {
      continue;
    }
    char request[4096];
    (void)read(conn, request, sizeof(request)); /* discard the queried path */
    char response[4200];
    int n = snprintf(response, sizeof(response), "%s\n", reply);
    if (n > 0) {
      (void)write(conn, response, (size_t)n);
    }
    close(conn);
  }
  return 0;
}
