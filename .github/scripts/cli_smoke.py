#!/usr/bin/env python3
"""Deterministic CLI/network-contract checks using only loopback and temporary data.

Run after `cargo build --locked --bin speedtest`. No public speed tests or DNS
configuration changes are performed. Unix additionally verifies PTY restoration.
"""
import argparse
import contextlib
import http.server
import json
import os
from pathlib import Path
import select
import signal
import socket
import subprocess
import tempfile
import threading
import time


class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *_args):
        pass

    def do_GET(self):
        if self.path.startswith("/slow/"):
            time.sleep(2)
        payload = b"x" * 65536 if "garbage.php" in self.path else b""
        if payload:
            time.sleep(0.015)  # Bound fixture traffic; these are not WAN benchmarks.
        self.send_response(200)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        with contextlib.suppress(BrokenPipeError, ConnectionResetError):
            self.wfile.write(payload)

    def do_POST(self):
        remaining = int(self.headers.get("Content-Length", "0"))
        self.connection.settimeout(3)
        try:
            while remaining:
                data = self.rfile.read(min(remaining, 65536))
                if not data:
                    return
                remaining -= len(data)
                time.sleep(0.005)
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()
        except (OSError, TimeoutError):
            pass  # A deadline intentionally disconnects an in-flight upload.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/speedtest.exe" if os.name == "nt" else "target/debug/speedtest")
    parser.add_argument("--output", default="verification/cli-smoke.txt")
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve(strict=True))
    transcript = []
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_port}"
    with tempfile.TemporaryDirectory(prefix="speedtest-smoke-") as scratch:
        env = os.environ.copy()
        env.update(HOME=scratch, XDG_DATA_HOME=scratch, LOCALAPPDATA=scratch,
                   NO_COLOR="1", TERM="dumb", NO_PROXY="*", no_proxy="*", RUST_BACKTRACE="0")

        def run(label, arguments, code=0, timeout=30):
            result = subprocess.run([binary, *arguments], env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, timeout=timeout)
            text = result.stdout.decode("utf-8", "replace")
            errors = result.stderr.decode("utf-8", "replace")
            transcript.append(f"## {label}\n$ speedtest {' '.join(arguments)}\nexit={result.returncode}\nstdout:\n{text}\nstderr:\n{errors}\n")
            assert result.returncode == code, (label, result.returncode, errors)
            assert b"\x1b" not in result.stdout and b"\x1b" not in result.stderr, label
            return result

        options = ["--backend", "librespeed", "--librespeed-server", base,
                   "--duration", "3", "--streams", "1", "--no-save"]
        try:
            human = run("redirected default automatically uses plain output", options)
            assert b"Download" in human.stdout and not human.stderr
            result_file = Path(scratch) / "result.json"
            structured = run("JSON result with explicit stderr progress", [*options, "--json", "--progress", "always", "--output", str(result_file)])
            result = json.loads(structured.stdout)
            assert result["download"]["bytes"] > 0 and result["upload"]["bytes"] > 0
            assert result["download"]["mbps"] > 0 and result["upload"]["mbps"] > 0
            assert structured.stderr and b"\r" not in structured.stderr
            for phase in ("download", "upload"):
                actual = result[phase]
                expected = actual["bytes"] * 8 / actual["seconds"] / 1_000_000
                assert abs(actual["mbps"] - expected) < 1e-8, phase
            assert json.loads(result_file.read_bytes()) == result
            assert not list(Path(scratch).rglob("history.jsonl")), "--no-save wrote history"
            run("offline threshold pass", ["check", str(result_file), "--min-download", "0", "--json"])
            failed = run("offline threshold failure", ["check", str(result_file), "--min-download", "1000000000", "--json"], 3)
            assert not json.loads(failed.stdout)["passed"] and not failed.stderr
            timed = run("bounded stalled endpoint", ["--backend", "librespeed", "--librespeed-server", base + "/slow/", "--timeout", "1", "--json", "--no-save"], 124, 5)
            assert not timed.stdout and json.loads(timed.stderr)["error"]["code"] == 124
            # Keep the port reserved without listening: deterministic connection refusal.
            with socket.socket() as reserved:
                reserved.bind(("127.0.0.1", 0))
                refused = run("LAN refusal is actionable", ["lan", f"127.0.0.1:{reserved.getsockname()[1]}", "--json"], 1, 10)
                assert not refused.stdout and json.loads(refused.stderr)["error"]["code"] == 1
            # Bind before announcing readiness; port zero also exercises the actual address.
            child = subprocess.Popen([binary, "serve", "--bind", "127.0.0.1:0"], env=env,
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                ready = child.stdout.readline().decode("utf-8") + child.stdout.readline().decode("utf-8")
                transcript.append("## LAN readiness\n" + ready)
                assert "127.0.0.1:" in ready, ready
                import re
                endpoint = re.search(r"127\.0\.0\.1:\d+", ready).group(0)
                lan = run("LAN client/server round trip", ["lan", endpoint, "--duration", "2", "--streams", "1", "--json"], 0, 15)
                measured = json.loads(lan.stdout)
                assert measured["download"]["bytes"] > 0 and measured["upload"]["bytes"] > 0
                bound = run("failed bind never announces readiness", ["serve", "--bind", endpoint], 1, 5)
                assert not bound.stdout
            finally:
                child.terminate()
                child.communicate(timeout=5)
            if os.name != "nt":
                child = subprocess.Popen([binary, "--backend", "librespeed", "--librespeed-server", base + "/slow/", "--json", "--no-save"], env=env,
                                         stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                time.sleep(0.25)
                child.send_signal(signal.SIGINT)
                out, err = child.communicate(timeout=5)
                assert child.returncode == 130 and not out, (child.returncode, err)
                assert json.loads(err)["error"]["code"] == 130
                transcript.append("## SIGINT\nexit=130; stdout empty; structured cancellation on stderr\n")
                pty_check(binary, options, env, transcript)
            transcript.append("PASS: all applicable loopback, output, cancellation, and terminal checks\n")
        finally:
            output.write_text("\n".join(transcript), encoding="utf-8")
            server.shutdown()
            server.server_close()
    print(transcript[-1].strip())


def pty_check(binary, options, env, transcript):
    import fcntl
    import pty
    import struct
    import termios
    master, slave = pty.openpty()
    original = termios.tcgetattr(slave)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 28, 100, 0, 0))
    tty_env = {k: v for k, v in env.items() if k not in ("NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE")}
    tty_env["TERM"] = "xterm-256color"
    # Isolate this PTY from any controlling terminal of the launching shell.
    child = subprocess.Popen([binary, "--run", *options], env=tty_env, stdin=slave,
                             stdout=slave, stderr=slave, start_new_session=True)
    data = bytearray()
    try:
        deadline = time.monotonic() + 5
        while b"\x1b[?1049h" not in data and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                data.extend(os.read(master, 65536))
        assert b"\x1b[?1049h" in data, bytes(data)
        os.write(master, b"q")
        while child.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                data.extend(os.read(master, 65536))
        child.wait(timeout=2)
        while select.select([master], [], [], 0.1)[0]:
            data.extend(os.read(master, 65536))
        restored = termios.tcgetattr(slave)
        assert child.returncode == 130, (child.returncode, bytes(data))
        assert restored == original, "PTY attributes were not restored"
        assert b"\x1b[?1049l" in data, "alternate screen was not restored"
        transcript.append("## Interactive PTY\nq exits 130; alternate screen left; all termios attributes restored\n")
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)


if __name__ == "__main__":
    main()
