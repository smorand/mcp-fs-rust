#!/usr/bin/env python3
"""Check the agent's line editor on a real pty.

Terminal geometry bugs are invisible to unit tests: a wrong prompt width still passes every
assertion about the width function, because the mistake is at the call site. This drives the
real binary through a pty, applies its output to a virtual screen, and reads back what a
human would see.

Self contained: it starts a server on an ephemeral port in a temp directory, mints its own
token, and needs no LLM key that works, because no prompt is ever submitted.

    python3 scripts/pty_check.py           run from the repository root
"""

import fcntl
import os
import pty
import re
import select
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time

COLS, ROWS = 80, 24
PROMPT = "\u276f"


class Screen:
    """Just enough VT100 to judge cursor placement and text layout."""

    def __init__(self, cols=COLS, rows=ROWS):
        self.cols, self.rows = cols, rows
        self.grid = [[" "] * cols for _ in range(rows)]
        self.r = self.c = 0

    def _clamp(self):
        self.r = max(0, min(self.rows - 1, self.r))
        self.c = max(0, min(self.cols - 1, self.c))

    def feed(self, data):
        i = 0
        while i < len(data):
            ch = data[i]
            if ch == "\x1b" and data[i : i + 2] == "\x1b[":
                m = re.match(r"\x1b\[([0-9;]*)([@-~])", data[i:])
                if not m:
                    i += 1
                    continue
                first = m.group(1).split(";")[0]
                n = int(first) if first else 1
                final = m.group(2)
                if final == "A":
                    self.r -= n
                elif final == "B":
                    self.r += n
                elif final == "G":
                    self.c = (n or 1) - 1
                elif final == "J":
                    for x in range(self.c, self.cols):
                        self.grid[self.r][x] = " "
                    for y in range(self.r + 1, self.rows):
                        self.grid[y] = [" "] * self.cols
                elif final == "K":
                    for x in range(self.c, self.cols):
                        self.grid[self.r][x] = " "
                self._clamp()
                i += m.end()
                continue
            if ch == "\r":
                self.c = 0
            elif ch == "\n":
                self.r += 1
            elif ch == "\x08":
                self.c -= 1
            elif ch >= " ":
                self.grid[self.r][self.c] = ch
                self.c += 1
                if self.c >= self.cols:
                    self.c = 0
                    self.r += 1
            self._clamp()
            i += 1

    def row(self, r):
        return "".join(self.grid[r]).rstrip()

    def prompt_row(self):
        """The last row showing the prompt glyph, which is the active input row."""
        for r in range(self.rows - 1, -1, -1):
            if PROMPT in self.row(r):
                return r
        return None

    def input_text(self):
        """Everything on the input area after the prompt glyph, following any wrap.

        A row is only continued when its last cell is occupied, which is what a wrap
        actually means. Joining every non empty row below would pick up unrelated output.
        """
        r = self.prompt_row()
        if r is None:
            return None
        head = "".join(self.grid[r])
        text = head[head.index(PROMPT) + 1 :]
        while self.grid[r][self.cols - 1] != " " and r + 1 < self.rows:
            r += 1
            text += "".join(self.grid[r])
        return text.strip()


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Harness:
    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="agent-pty-")
        self.port = free_port()
        self.server = None
        self.fd = None
        self.pid = None
        self.screen = Screen()
        self.raw = []

    def start_server(self):
        keys = os.path.join(self.dir, "keys")
        os.makedirs(keys)
        subprocess.run(
            ["./target/debug/mcp-fs", "keys", "--dir", keys],
            check=True,
            capture_output=True,
        )
        cfg = os.path.join(self.dir, "cfg.yaml")
        with open(cfg, "w") as f:
            f.write(
                f"server: {{ host: 127.0.0.1, port: {self.port} }}\n"
                f"auth: {{ jwt: {{ public_key_path: {keys}/jwt.pub, issuer: web-a2a,"
                f" username_claim: email }}, admins: [pty@example.com] }}\n"
                f"infra: {{ meta: {{ dir: {self.dir}/volumes }},"
                f" blob: {{ dir: {self.dir}/blobs }},"
                f" admin: {{ path: {self.dir}/admin.db }} }}\n"
            )
        self.server = subprocess.Popen(
            ["./target/debug/mcp-fs", "serve", "--config", cfg],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        token = subprocess.run(
            [
                "./target/debug/mcp-fs",
                "token",
                "pty@example.com",
                "--key",
                f"{keys}/jwt.key",
                "--ttl",
                "600",
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        tokens_dir = os.path.join(self.dir, "tokens")
        os.makedirs(tokens_dir)
        with open(os.path.join(tokens_dir, "pty"), "w") as f:
            f.write(token)

        self.agent_cfg = os.path.join(self.dir, "agent.yaml")
        with open(self.agent_cfg, "w") as f:
            f.write(
                f"mcp:\n  url: http://127.0.0.1:{self.port}/mcp\n"
                f"  tokens_dir: {tokens_dir}\n"
                "llm:\n  api_key: unused-no-prompt-is-submitted\n"
                "  api_key_env: PTY_CHECK_NO_SUCH_VAR\n"
            )
        # Wait for the port to answer.
        for _ in range(100):
            try:
                with socket.create_connection(("127.0.0.1", self.port), 0.2):
                    return
            except OSError:
                time.sleep(0.1)
        raise RuntimeError("server never came up")

    def start_agent(self, history=()):
        # The agent keeps its history beside the working directory, so run it inside the
        # temp dir: the repository stays clean and the history can be seeded, which lets
        # the recall checks run without ever submitting a line to the LLM.
        hist_dir = os.path.join(self.dir, ".agent_history")
        os.makedirs(hist_dir, exist_ok=True)
        with open(os.path.join(hist_dir, "readline.txt"), "w") as f:
            for line in history:
                f.write(line + "\n")

        agent_bin = os.path.abspath("./target/debug/agent")
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.chdir(self.dir)
            os.execv(agent_bin, ["agent", "--user", "pty", "--config", self.agent_cfg])
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.pump(8.0, until="Connected")
        if "Connected" not in "".join(self.raw):
            raise RuntimeError("agent never connected:\n" + "".join(self.raw))
        self.pump(0.6)

    def pump(self, seconds, until=None):
        end = time.time() + seconds
        while time.time() < end:
            if until and until in "".join(self.raw):
                return
            r, _, _ = select.select([self.fd], [], [], 0.15)
            if self.fd in r:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    return
                if not chunk:
                    return
                text = chunk.decode("utf8", "replace")
                self.raw.append(text)
                self.screen.feed(text)

    def send(self, data, settle=0.3):
        os.write(self.fd, data)
        time.sleep(settle)
        self.pump(0.25)

    def clear_line(self):
        """Empty the input area: End, then kill backwards.

        Ctrl+U kills from the cursor to the start, like readline, so it only empties the
        line when the cursor is already at the end.
        """
        self.send(b"\x05")  # Ctrl+E
        self.send(b"\x15")  # Ctrl+U

    def close(self):
        try:
            if self.fd is not None:
                self.send(b"\x03")
                self.send(b"exit\r")
                self.pump(1.5)
                os.close(self.fd)
        except OSError:
            pass
        if self.pid:
            try:
                os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                pass
        if self.server:
            self.server.terminate()
            self.server.wait(timeout=10)
        shutil.rmtree(self.dir, ignore_errors=True)


def main():
    if not os.path.exists("./target/debug/agent"):
        print("build first: cargo build -p agent -p mcp-fs", file=sys.stderr)
        return 2

    h = Harness()
    failures = []

    def check(name, got, want):
        if got == want:
            print(f"  ok    {name}")
        else:
            print(f"  FAIL  {name}\n          got  {got!r}\n          want {want!r}")
            failures.append(name)

    try:
        h.start_server()
        h.start_agent(history=["remembered line"])

        # A backspace must not shift the text: with the prompt's colour escapes measured
        # as printable cells, the repositioning landed seven columns too far right and left
        # an unerasable gap that only appeared once typing resumed.
        h.send(b"abcdef")
        h.send(b"\x7f\x7f")
        check("backspace leaves no gap", h.screen.input_text(), "abcd")
        h.send(b"XY")
        check("typing resumes on the text", h.screen.input_text(), "abcdXY")

        # Ctrl+U kills from the cursor to the start, Ctrl+K from the cursor to the end.
        h.send(b"\x15")
        check("ctrl+u kills back to the start", h.screen.input_text(), "")
        h.send(b"hello world")
        h.send(b"\x01")  # Ctrl+A, back to the start
        h.send(b"\x0b")  # Ctrl+K, kill to the end
        check("ctrl+a then ctrl+k empties it", h.screen.input_text(), "")

        # Editing in the middle of the line.
        h.clear_line()
        h.send(b"abcd")
        h.send(b"\x1b[D\x1b[D")  # two lefts
        h.send(b"Z")
        check("insert in the middle", h.screen.input_text(), "abZcd")
        h.send(b"\x7f")
        check("backspace in the middle", h.screen.input_text(), "abcd")

        # Across the wrap boundary: a line longer than the terminal, then a backspace.
        h.clear_line()
        long_text = b"L" * 95
        h.send(long_text, settle=0.8)
        check("a wrapped line is intact", h.screen.input_text(), "L" * 95)
        h.send(b"\x7f\x7f")
        check("backspace across the wrap", h.screen.input_text(), "L" * 93)

        # History navigation comes last, and this first recall is the interesting one: the
        # cursor sits on the FIRST row of a wrapped line. The reference measured the rewind
        # from the old end here, so it went up one row too many and painted over the line
        # above. Repeated Up at the oldest entry is a no op, which is why this case has to
        # use the first Up of the session.
        h.send(b"\x01")  # Home, cursor on row 0 while the content spans two rows
        row_before = h.screen.prompt_row()
        h.send(b"\x1b[A")
        check("recall from home on a wrapped line", h.screen.input_text(), "remembered line")
        # The repainted text can look right while sitting one row too high, which silently
        # overwrites whatever was printed above. Pin the absolute row: measuring the rewind
        # from the old end moves the whole input area up by one.
        check("the recall stays on its own row", h.screen.prompt_row(), row_before)

        # Down returns to the line that was being edited, which was stashed on the way up.
        h.send(b"\x1b[B")
        check("down restores the stashed line", h.screen.input_text(), "L" * 93)
        h.send(b"\x1b[A")
        check("history recall", h.screen.input_text(), "remembered line")
    finally:
        h.close()

    print()
    if failures:
        print(f"FAILED: {len(failures)} check(s): {', '.join(failures)}")
        return 1
    print("all editor checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
