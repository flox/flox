#!/usr/bin/env python3
"""A minimal OAuth 2.0 device-grant server for testing 'flox auth login'.

Usage: device_grant_server.py <PORT_FILE> <ACCESS_TOKEN>

Listens on a free localhost port and writes it to PORT_FILE.

  POST /device   issues a device code ("device-N", user code "CODE-N")
  POST /token    answers 'authorization_pending' until the code is approved,
                 then returns ACCESS_TOKEN
  GET  /approve  approves every code issued so far
  GET  /issued   the number of device codes issued so far

Every other path answers 404, which 'flox auth login' reads as a FloxHub that
serves no login configuration.
"""

import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs

PORT_FILE, ACCESS_TOKEN = sys.argv[1], sys.argv[2]

lock = threading.Lock()
issued = 0
approved = set()


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body):
        payload = json.dumps(body).encode() if not isinstance(body, str) else body.encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        with lock:
            if self.path == "/approve":
                approved.update(f"device-{n}" for n in range(1, issued + 1))
                return self.reply(200, {})
            if self.path == "/issued":
                return self.reply(200, str(issued))
        self.reply(404, {})

    def do_POST(self):
        global issued
        length = int(self.headers.get("Content-Length", 0))
        form = parse_qs(self.rfile.read(length).decode())
        with lock:
            if self.path == "/device":
                issued += 1
                user_code = f"CODE-{issued}"
                base = f"http://127.0.0.1:{self.server.server_port}"
                return self.reply(200, {
                    "device_code": f"device-{issued}",
                    "user_code": user_code,
                    "verification_uri": f"{base}/activate",
                    "verification_uri_complete": f"{base}/activate?user_code={user_code}",
                    "expires_in": 900,
                    "interval": 1,
                })
            if self.path == "/token":
                if form.get("device_code", [""])[0] in approved:
                    return self.reply(200, {
                        "access_token": ACCESS_TOKEN,
                        "token_type": "bearer",
                        "expires_in": 3600,
                    })
                return self.reply(400, {"error": "authorization_pending"})
        self.reply(404, {})

    def log_message(self, format, *args):
        pass


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
# Rename into place so a reader never sees a partly written port.
with open(f"{PORT_FILE}.tmp", "w") as f:
    f.write(str(server.server_port))
os.rename(f"{PORT_FILE}.tmp", PORT_FILE)
server.serve_forever()
