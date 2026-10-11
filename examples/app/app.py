"""A tiny app for trying `ssp host`: it says which variant served the
request, and prints a `visit` outcome for every subject it serves."""

import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        subject = self.headers.get("x-statespace-subject")
        body = json.dumps(
            {
                "variant": os.environ.get("STATESPACE_VARIANT"),
                "subject": subject,
                "greeting": os.environ.get("GREETING", "hello"),
            }
        ).encode()
        if subject:
            print(json.dumps({"statespace": "visit", "subject": subject, "path": self.path}), flush=True)
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        print(f"{self.command} {self.path}", file=sys.stderr, flush=True)


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
