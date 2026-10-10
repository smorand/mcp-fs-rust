#!/usr/bin/env python3
"""A minimal stand in for a document service in `api` mode.

It implements the contract `ApiDocService` speaks, and nothing else:

    POST <any path>
    Content-Type: multipart/form-data with one part named by --file-field (default "file")
    optional auth header, checked by name and exact value
    -> 200 text/markdown, the body IS the Markdown
    -> with --bundle: 200 application/json, the SPEC-0019 bundle
       {"markdown": "...", "files": {"<relative path>": "<base64>"}, "manifest": {...}}

The conversion itself is delegated to `doc-convert --stdout --quiet` (or, with
--bundle, `doc-convert --quiet -o <outdir>`) run inside a throwaway directory, so this server is also the reference for what a real
implementation has to do: stage the upload under its original name, convert,
answer with the Markdown.

It exists so the API mode can be exercised end to end without deploying anything,
and so a developer can watch what the server actually sends. It is single
threaded, keeps nothing, and is NOT a production component.

Usage:
    scripts/doc_service_fake.py --port 8099
    scripts/doc_service_fake.py --port 8099 --auth-header X-Convert-Key --auth-token s3cret
    scripts/doc_service_fake.py --port 8099 --bundle

Stdlib only: no dependency to install, so a functional test can just run it.
"""

import argparse
import base64
import email.parser
import email.policy
import json
import os
import shutil
import subprocess
import sys
import tempfile
from http.server import BaseHTTPRequestHandler, HTTPServer

# The part name a real endpoint expects is its own business, so it is settable
# here too and mirrors doc_service.api.file_field on the server side.
FILE_PART = "file"

# Enough for a large deck or a video, and a ceiling so a stray client cannot make
# the process eat the machine's memory.
MAX_BODY_BYTES = 512 * 1024 * 1024


def convert(file_name, payload):
    """Run the converter on `payload` in a sandbox and return its Markdown.

    The directory is removed on every path, including a converter that writes
    beside its input, which is exactly what the server does in cli mode.
    """
    workdir = tempfile.mkdtemp(prefix="doc-service-fake-")
    try:
        # A single path segment, so a crafted filename cannot climb out.
        safe = os.path.basename(file_name) or "document"
        staged = os.path.join(workdir, safe)
        with open(staged, "wb") as handle:
            handle.write(payload)
        result = subprocess.run(
            ["doc-convert", "--stdout", "--quiet", "./" + safe],
            cwd=workdir,
            capture_output=True,
        )
        if result.returncode != 0:
            tail = result.stderr[-2048:].decode("utf-8", "replace").strip()
            raise RuntimeError("doc-convert exited with %d: %s" % (result.returncode, tail))
        return result.stdout
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def convert_bundle(file_name, payload):
    """Run the converter in bundle mode and return the JSON bundle as bytes.

    `document.md` is the Markdown, every other regular file under the output
    directory travels base64 encoded by its relative path, and `artifacts.json`,
    when the converter wrote one, is the manifest. Symlinks are not followed.
    """
    workdir = tempfile.mkdtemp(prefix="doc-service-fake-")
    try:
        safe = os.path.basename(file_name) or "document"
        with open(os.path.join(workdir, safe), "wb") as handle:
            handle.write(payload)
        outdir = os.path.join(workdir, ".bundle")
        os.mkdir(outdir)
        result = subprocess.run(
            ["doc-convert", "--quiet", "-o", outdir, "./" + safe],
            cwd=workdir,
            capture_output=True,
        )
        if result.returncode != 0:
            tail = result.stderr[-2048:].decode("utf-8", "replace").strip()
            raise RuntimeError("doc-convert exited with %d: %s" % (result.returncode, tail))
        files = {}
        for root, _dirs, names in os.walk(outdir):
            for name in names:
                path = os.path.join(root, name)
                if os.path.islink(path) or not os.path.isfile(path):
                    continue
                with open(path, "rb") as handle:
                    key = os.path.relpath(path, outdir).replace(os.sep, "/")
                    files[key] = handle.read()
        if "document.md" not in files:
            raise RuntimeError("doc-convert wrote no document.md")
        bundle = {
            "markdown": files.pop("document.md").decode("utf-8", "replace"),
            "files": {k: base64.b64encode(v).decode("ascii") for k, v in files.items()},
        }
        if "artifacts.json" in files:
            bundle["manifest"] = json.loads(files["artifacts.json"])
        return json.dumps(bundle).encode("utf-8")
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def parse_file_part(content_type, body):
    """Return `(filename, bytes)` of the part named `file`, or `None`."""
    # The stdlib email parser understands multipart/form-data as long as it is
    # handed the Content-Type header along with the body.
    raw = b"Content-Type: " + content_type.encode() + b"\r\n\r\n" + body
    message = email.parser.BytesParser(policy=email.policy.HTTP).parsebytes(raw)
    for part in message.walk():
        disposition = part.get("Content-Disposition", "")
        if "form-data" not in disposition:
            continue
        if part.get_param("name", header="content-disposition") != FILE_PART:
            continue
        name = part.get_param("filename", header="content-disposition") or "document"
        return name, part.get_payload(decode=True) or b""
    return None


class Handler(BaseHTTPRequestHandler):
    """One request, one conversion. The server settings live on the class."""

    auth_header = ""
    auth_token = ""
    bundle = False

    def log_message(self, fmt, *args):
        sys.stderr.write("doc-service-fake: " + (fmt % args) + "\n")

    def reply(self, status, body, content_type="text/plain; charset=utf-8"):
        payload = body if isinstance(body, bytes) else body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        # A liveness probe, so a test can wait for the port to answer.
        self.reply(200, "doc-service-fake ready\n")

    def do_POST(self):
        if self.auth_token:
            seen = self.headers.get(self.auth_header)
            if seen != self.auth_token:
                self.reply(401, "bad or missing %s\n" % self.auth_header)
                return

        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0 or length > MAX_BODY_BYTES:
            self.reply(400, "expected a body of 1 to %d bytes\n" % MAX_BODY_BYTES)
            return
        content_type = self.headers.get("Content-Type") or ""
        if "multipart/form-data" not in content_type:
            self.reply(400, "expected multipart/form-data\n")
            return

        part = parse_file_part(content_type, self.rfile.read(length))
        if part is None:
            self.reply(400, "no part named '%s'\n" % FILE_PART)
            return

        name, payload = part
        try:
            if self.bundle:
                self.reply(200, convert_bundle(name, payload), "application/json")
                return
            markdown = convert(name, payload)
        except FileNotFoundError:
            self.reply(500, "doc-convert is not on PATH\n")
            return
        except RuntimeError as err:
            self.reply(500, str(err) + "\n")
            return
        self.reply(200, markdown, "text/markdown; charset=utf-8")


def main():
    global FILE_PART
    parser = argparse.ArgumentParser(
        description="A fake document service speaking the api mode contract of mcp-fs.",
    )
    parser.add_argument("--port", type=int, default=8099, help="port to listen on (default 8099)")
    parser.add_argument(
        "--file-field",
        default=FILE_PART,
        help="name of the multipart part carrying the document (default file)",
    )
    parser.add_argument(
        "--auth-header",
        default="Authorization",
        help="name of the header carrying the token (default Authorization)",
    )
    parser.add_argument(
        "--auth-token",
        default="",
        help="expected header value, verbatim. Empty means no authentication.",
    )
    parser.add_argument(
        "--bundle",
        action="store_true",
        help="answer the SPEC-0019 JSON bundle (markdown, files, manifest) instead of Markdown",
    )
    args = parser.parse_args()

    FILE_PART = args.file_field
    Handler.auth_header = args.auth_header
    Handler.auth_token = args.auth_token
    Handler.bundle = args.bundle
    # Loopback only: this server runs arbitrary conversions and has no business
    # being reachable from anywhere else.
    server = HTTPServer(("127.0.0.1", args.port), Handler)
    sys.stderr.write("doc-service-fake: listening on http://127.0.0.1:%d\n" % args.port)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
