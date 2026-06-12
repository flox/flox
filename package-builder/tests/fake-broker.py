#!/usr/bin/env python3
"""fake-broker.py — a scripted stand-in for the ask broker, for C probe tests.

The real broker is a thread inside the flox-activations executive. To test the
libsandbox RPC client (package-builder/sandbox.c `ask_broker`) in isolation,
this script binds the same AF_UNIX/SOCK_STREAM verdict socket and replies with
a canned verdict per the newline-JSON wire protocol:

  request line  (from libsandbox): {"v":1,"kind":"fs","op":"read|write",
                                    "path":..,"raw":..,"pid":..,"exe":..}
  response line (to libsandbox):   {"v":1,"verdict":"allow|deny",
                                    "scope":..,"cache":"scope|ttl|none",
                                    "req":<n>}

One request/response exchange per connection, matching the C client.

Every request is appended (as its raw JSON line) to a log file so a test can
assert the RPC count — in particular that a cached allow scope makes ZERO
further RPCs.

Usage:
  fake-broker.py --socket PATH --log PATH --mode MODE [--scope GLOB]

Modes:
  allow-scope   reply allow with cache=scope and the given --scope glob (so the
                engine caches the subtree and a second open under it never RPCs)
  deny          reply deny with cache=ttl and scope=<request path>, req counter
  allow-file    reply allow with cache=scope but scope=<request path> (a single
                file allowed, no subtree)

The mode can be switched live by writing a new mode word into the file named by
--mode-file (if given); this lets a test flip deny -> allow to exercise the
2-second negative TTL.
"""

import argparse
import json
import os
import socketserver
import sys
import threading


class State:
    """Shared, mutable broker behavior; guarded by a lock."""

    def __init__(self, mode, scope, log_path, mode_file):
        self.lock = threading.Lock()
        self.mode = mode
        self.scope = scope
        self.log_path = log_path
        self.mode_file = mode_file
        self.req = 0

    def current_mode(self):
        # A test can flip the mode mid-run by writing a word into mode_file
        # (used to prove the negative-TTL expiry: deny, then allow on retry).
        if self.mode_file and os.path.exists(self.mode_file):
            with open(self.mode_file) as handle:
                word = handle.read().strip()
                if word:
                    return word
        return self.mode

    def next_req(self):
        with self.lock:
            self.req += 1
            return self.req

    def log_request(self, line):
        with self.lock:
            with open(self.log_path, "a") as handle:
                handle.write(line + "\n")


def make_handler(state):
    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            data = b""
            # Read one line (the request is a single newline-terminated JSON
            # object); stop at the newline.
            while b"\n" not in data:
                chunk = self.request.recv(4096)
                if not chunk:
                    break
                data += chunk
            line = data.decode("utf-8", "replace").strip()
            if not line:
                return
            state.log_request(line)
            try:
                request = json.loads(line)
            except json.JSONDecodeError:
                request = {}
            path = request.get("path", "")

            mode = state.current_mode()
            if mode == "allow-scope":
                response = {
                    "v": 1,
                    "verdict": "allow",
                    "scope": state.scope or path,
                    "cache": "scope",
                    "req": 0,
                }
            elif mode == "allow-file":
                response = {
                    "v": 1,
                    "verdict": "allow",
                    "scope": path,
                    "cache": "scope",
                    "req": 0,
                }
            else:  # deny
                response = {
                    "v": 1,
                    "verdict": "deny",
                    "scope": path,
                    "cache": "ttl",
                    "req": state.next_req(),
                }
            self.request.sendall((json.dumps(response) + "\n").encode("utf-8"))

    return Handler


class Server(socketserver.ThreadingUnixStreamServer):
    # Allow quick rebind in case a prior run left the path around.
    allow_reuse_address = True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--mode", required=True,
                        choices=["allow-scope", "allow-file", "deny"])
    parser.add_argument("--scope", default=None)
    parser.add_argument("--mode-file", default=None)
    args = parser.parse_args()

    # Fresh log and socket each run.
    open(args.log, "w").close()
    if os.path.exists(args.socket):
        os.unlink(args.socket)

    state = State(args.mode, args.scope, args.log, args.mode_file)
    server = Server(args.socket, make_handler(state))
    os.chmod(args.socket, 0o600)

    # Signal readiness on stdout so the test can wait for the socket to exist
    # before launching the probe.
    print("READY", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        if os.path.exists(args.socket):
            os.unlink(args.socket)


if __name__ == "__main__":
    sys.exit(main())
