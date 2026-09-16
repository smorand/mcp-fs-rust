#!/usr/bin/env python3
"""Fake embedding server for tests. Returns a fixed float[N] vector.

Usage:
    python3 scripts/search_embedding_fake.py --port 8098 --dims 1536

Returns {"data": [{"embedding": [0.1] * dims, "index": 0}], "model": "fake",
         "usage": {"prompt_tokens": 1, "total_tokens": 1}}
for any POST to /v1/embeddings.
"""

import argparse
import json
from http.server import BaseHTTPRequestHandler, HTTPServer


def make_handler(dims: int):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            # Suppress the default access log for cleaner test output.
            pass

        def do_POST(self):
            content_length = int(self.headers.get("Content-Length", 0))
            _ = self.rfile.read(content_length)

            embedding = [0.1] * dims
            body = json.dumps({
                "data": [{"embedding": embedding, "index": 0}],
                "model": "fake",
                "usage": {"prompt_tokens": 1, "total_tokens": 1},
            }).encode()

            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    return Handler


def main():
    parser = argparse.ArgumentParser(description="Fake embedding server for tests")
    parser.add_argument("--port", type=int, default=8098, help="Port to listen on")
    parser.add_argument("--dims", type=int, default=1536, help="Embedding dimensions")
    args = parser.parse_args()

    server = HTTPServer(("127.0.0.1", args.port), make_handler(args.dims))
    print(f"Fake embedding server listening on http://127.0.0.1:{args.port} (dims={args.dims})")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
