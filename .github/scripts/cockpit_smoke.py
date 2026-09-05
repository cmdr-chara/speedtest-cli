#!/usr/bin/env python3
"""Real-PTY cockpit checks. Only local fixtures; no public tests or DNS writes.

Run after cargo build. Unix exercises the actual executable, termios restoration,
resize, offline startup, diagnostic subprocesses, measurement completion and retry.
Windows retains the platform-independent Rust state/render and CLI contract tests.
"""
import argparse
import codecs
import contextlib
import csv
import datetime
import http.server
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unicodedata

from cli_smoke import Fixture


class CountedFixture(Fixture):
    def do_CONNECT(self):
        # An explicit local HTTPS proxy for cancellation tests. Never tunnel to WAN.
        with self.server.count_lock:
            self.server.requests += 1
        self.send_response(503)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def do_GET(self):
        with self.server.count_lock:
            self.server.requests += 1
        if self.path.startswith('/fail/'):
            self.send_response(503)
            self.send_header('Content-Length', '0')
            self.end_headers()
        else:
            super().do_GET()

    def do_POST(self):
        with self.server.count_lock:
            self.server.requests += 1
        super().do_POST()


class Screen:
    """Small VT screen observer for Crossterm cursor/erase operations, not OCR."""
    def __init__(self, width=80, height=24):
        self.width, self.height = width, height
        self.cells = [[' '] * width for _ in range(height)]
        self.x = self.y = 0
        self.pending = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')

    def feed(self, data):
        text = self.pending + self.decoder.decode(data)
        self.pending = ''
        i = 0
        while i < len(text):
            ch = text[i]
            if ch == '\x1b':
                if i + 1 >= len(text):
                    self.pending = text[i:]
                    break
                if text[i + 1] == '[':
                    match = re.match(r'\x1b\[([0-?]*)([ -/]*)([@-~])', text[i:])
                    if not match:
                        self.pending = text[i:]
                        break
                    raw, _, code = match.groups()
                    private = raw.startswith('?')
                    values = [int(v) if v else 0 for v in raw.lstrip('?').split(';')]
                    n = values[0] or 1
                    if code in ('H', 'f'):
                        self.y = n - 1
                        self.x = (values[1] if len(values) > 1 and values[1] else 1) - 1
                    elif code == 'A': self.y -= n
                    elif code == 'B': self.y += n
                    elif code == 'C': self.x += n
                    elif code == 'D': self.x -= n
                    elif code == 'G': self.x = n - 1
                    elif code == 'd': self.y = n - 1
                    elif code == 'J' and values[0] in (2, 3):
                        self.cells = [[' '] * self.width for _ in range(self.height)]
                    elif code == 'K' and 0 <= self.y < self.height:
                        start, end = (0, self.width) if values[0] == 2 else (self.x, self.width)
                        for x in range(max(0, start), end): self.cells[self.y][x] = ' '
                    elif private and code == 'h' and 1049 in values:
                        self.cells = [[' '] * self.width for _ in range(self.height)]
                        self.x = self.y = 0
                    i += len(match.group(0))
                    continue
                # Ignore non-CSI terminal mode changes; the app does not emit OSC.
                i += 2
                continue
            if ch == '\r': self.x = 0
            elif ch == '\n': self.y += 1
            elif ch == '\t': self.x = (self.x // 8 + 1) * 8
            elif not unicodedata.category(ch).startswith('C'):
                width = 0 if unicodedata.combining(ch) else (2 if unicodedata.east_asian_width(ch) in 'WF' else 1)
                if 0 <= self.y < self.height and 0 <= self.x < self.width:
                    self.cells[self.y][self.x] = ch
                    if width == 2 and self.x + 1 < self.width: self.cells[self.y][self.x + 1] = ''
                self.x += width
            i += 1

    def text(self):
        return '\n'.join(''.join(row) for row in self.cells)


class Tty:
    def __init__(self, binary, arguments, env, transcript):
        import fcntl
        import pty
        import struct
        import termios
        self.termios = termios
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        self.screen = Screen()
        self.raw = bytearray()
        self.transcript = transcript
        self.process = subprocess.Popen([binary, *arguments], env=env, stdin=self.slave, stdout=self.slave, stderr=self.slave)

    def pump(self, duration=0.1):
        until = time.monotonic() + duration
        while time.monotonic() < until:
            if select.select([self.master], [], [], min(0.05, max(0, until - time.monotonic())))[0]:
                data = os.read(self.master, 65536)
                self.raw.extend(data)
                self.screen.feed(data)

    def wait(self, text, timeout=6):
        until = time.monotonic() + timeout
        while time.monotonic() < until:
            self.pump()
            if text in self.screen.text(): return
            if self.process.poll() is not None: break
        raise AssertionError(f'missing {text!r}; exit={self.process.poll()}\n{self.screen.text()}')

    def send(self, value):
        os.write(self.master, value.encode())
        self.pump(0.12)

    def resize(self, width, height):
        import fcntl
        import struct
        self.screen = Screen(width, height)
        fcntl.ioctl(self.slave, self.termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
        self.process.send_signal(signal.SIGWINCH)
        self.pump(0.2)

    def snapshot(self, label):
        self.transcript.append(f'## {label}\n{self.screen.text()}\n')

    def finish(self, code=0, alternate=True):
        until = time.monotonic() + 5
        while self.process.poll() is None and time.monotonic() < until:
            self.pump()
        self.process.wait(timeout=1)
        self.pump(0.1)
        assert self.process.returncode == code, (self.process.returncode, self.screen.text())
        assert self.termios.tcgetattr(self.slave) == self.original, 'termios attributes changed'
        if alternate:
            assert b'\x1b[?1049h' in self.raw and b'\x1b[?1049l' in self.raw, 'alternate screen not restored'
        else:
            assert b'\x1b[?1049h' not in self.raw, 'noninteractive command entered alternate screen'

    def close(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=5)
        os.close(self.master)
        os.close(self.slave)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/speedtest.exe' if os.name == 'nt' else 'target/debug/speedtest')
    parser.add_argument('--output', default='verification/cockpit-smoke.txt')
    args = parser.parse_args()
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    if os.name == 'nt':
        output.write_text('SKIPPED: Unix PTY checks; Rust navigation/render and existing CLI checks run on Windows.\n', encoding='utf-8')
        print(output.read_text().strip())
        return
    binary = str(Path(args.binary).resolve(strict=True))
    transcript = []
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), CountedFixture)
    server.daemon_threads = True
    server.requests = 0
    server.count_lock = threading.Lock()
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f'http://127.0.0.1:{server.server_port}'
    try:
        with tempfile.TemporaryDirectory(prefix='cockpit-smoke-') as scratch:
            env = {k: v for k, v in os.environ.items() if k not in ('NO_COLOR', 'CLICOLOR', 'CLICOLOR_FORCE')}
            env.update(HOME=scratch, XDG_DATA_HOME=scratch, LOCALAPPDATA=scratch,
                       TERM='xterm-256color', COLORTERM='truecolor', NO_PROXY='*', no_proxy='*')
            options = ['--backend', 'librespeed', '--librespeed-server', base, '--duration', '3', '--streams', '1', '--timeout', '20', '--no-save']
            root = Path(scratch) / ('Library/Application Support/speedtest' if sys.platform == 'darwin' else 'speedtest')
            result_path = Path(scratch) / 'completed.json'

            @contextlib.contextmanager
            def session(arguments=options, environment=None):
                tty = Tty(binary, arguments, environment or env, transcript)
                try:
                    yield tty
                except BaseException:
                    tty.snapshot('FAILURE screen')
                    raise
                finally:
                    tty.close()

            with session([]) as tty:
                tty.wait('No tests yet')
                tty.wait('NETWORK NOT PROBED')
                tty.snapshot('Bare speedtest / 80x24 home')
                tty.send('q')
                tty.finish()
            assert server.requests == 0

            with session(options) as tty:
                tty.wait('No tests yet')
                tty.send('\r')
                tty.wait('READY WHEN YOU ARE')
                tty.send('?')
                tty.wait('KEYBOARD FIELD GUIDE')
                tty.send('\x1b')
                tty.send('\x1b')
                tty.wait('No tests yet')
                for label in ['READY WHEN YOU ARE', 'YOUR NETWORK, OVER TIME', 'THE BIGGER PICTURE', 'DNS WORKBENCH']:
                    tty.send('\t')
                    tty.wait(label)
                tty.send('j')  # Offline resolver catalog.
                tty.send('\r')
                tty.wait('READY TO START')
                assert server.requests == 0
                tty.send('\r')
                tty.wait('DNS PROVIDERS')
                tty.snapshot('Existing DNS catalog command rendered in cockpit')
                tty.send('q')
                tty.finish()
            assert server.requests == 0, 'navigation or local tool sent measurement requests'

            short = options.copy()
            short[short.index('--timeout') + 1] = '1'
            with session(short) as tty:
                tty.wait('No tests yet')
                tty.pump(1.3)  # Deadline must apply to tests, never to menu dwell time.
                assert tty.process.poll() is None
                tty.resize(60, 15)
                tty.wait('TERMINAL TOO SMALL')
                tty.send('\r\r')
                assert server.requests == 0, 'hidden controls started a test'
                tty.resize(80, 24)
                tty.wait('No tests yet')
                tty.send('q')
                tty.finish()

            with session([*options, '--output', str(result_path)]) as tty:
                tty.wait('No tests yet')
                before = server.requests
                tty.send('\r')
                tty.wait('READY WHEN YOU ARE')
                tty.pump(0.3)
                assert server.requests == before
                tty.send('\r')
                tty.wait('MEASURING')
                tty.wait('MEASUREMENT COMPLETE', timeout=25)
                tty.wait('EXPORTED')
                tty.snapshot('Completed loopback measurement and export')
                result = json.loads(result_path.read_text())
                assert result['download']['bytes'] > 0 and result['upload']['bytes'] > 0
                assert not (root / 'history.jsonl').exists(), '--no-save persisted history'
                tty.send('\x1b')
                tty.wait('READY WHEN YOU ARE')
                tty.send('\x1b')
                tty.wait('LATEST RESULT')
                tty.send('v')
                tty.wait('MEASUREMENT COMPLETE')
                tty.send('q')
                tty.finish()

            # The same completion policy must export CSV and persist exactly once;
            # opening Results again is navigation, never a second save.
            csv_path = Path(scratch) / 'completed.csv'
            saving = [option for option in options if option != '--no-save']
            with session([*saving, '--output', str(csv_path), '--format', 'csv']) as tty:
                tty.wait('No tests yet')
                tty.send('\r\r')
                tty.wait('MEASUREMENT COMPLETE', timeout=25)
                tty.wait('SAVED to local history')
                history_file = root / 'history.jsonl'
                saved_lines = history_file.read_text(encoding='utf-8').splitlines()
                assert len(saved_lines) == 1, 'completion saved more than once'
                with csv_path.open(encoding='utf-8', newline='') as stream:
                    rows = list(csv.DictReader(stream))
                assert len(rows) == 1 and float(rows[0]['download_mbps']) > 0
                tty.send('\x1b')
                tty.wait('READY WHEN YOU ARE')
                tty.send('\x1b')
                tty.wait('LATEST RESULT')
                tty.send('v')
                tty.wait('MEASUREMENT COMPLETE')
                assert history_file.read_text(encoding='utf-8').splitlines() == saved_lines
                tty.snapshot('Shared completion policy / CSV export and exactly one history row')
                tty.send('q')
                tty.finish()
            # Only remove the fixture's own automatically persisted data.
            history_file.unlink()

            with session(options) as tty:
                tty.wait('No tests yet')
                tty.send('\r\r')
                tty.wait('MEASURING')
                tty.send('?')
                tty.wait('KEYBOARD FIELD GUIDE')
                tty.send('\x1b')
                tty.send('\x1b')
                tty.wait('CONFIRM CANCELLATION')
                tty.send('\r')  # Continue is the default.
                assert tty.process.poll() is None
                tty.send('\x1b')
                tty.wait('CONFIRM CANCELLATION')
                tty.send('y')
                tty.wait('READY WHEN YOU ARE')
                count = server.requests
                tty.pump(0.5)
                assert server.requests == count, 'cancelled engine kept sending requests'
                assert not (root / 'history.jsonl').exists()
                tty.send('q')
                tty.finish()

            failing = options.copy()
            failing[failing.index('--librespeed-server') + 1] = base + '/fail/'
            with session(failing) as tty:
                tty.wait('No tests yet')
                tty.send('\r\r')
                tty.wait('TEST COULD NOT COMPLETE')
                before = server.requests
                tty.snapshot('Failed backend / retry state')
                tty.send('r')
                tty.wait('TEST COULD NOT COMPLETE')
                assert server.requests > before, 'retry did not run'
                tty.send('\x1b')
                tty.wait('READY WHEN YOU ARE')
                tty.send('q')
                tty.finish()

            with session([*failing, '--plain']) as tty:
                tty.finish(1, alternate=False)
                assert b'NETWORK COCKPIT' not in tty.raw
                assert b'503' in tty.raw
                transcript.append('PASS: --plain on an actual terminal never enters the alternate screen.\n')

            # A diagnostic child probes only a local rejecting proxy. Cancellation must
            # stop its real HTTP traffic, not merely hide the UI or detach the child.
            proxy_env = dict(env, HTTPS_PROXY=base, https_proxy=base, HTTP_PROXY=base,
                             http_proxy=base, ALL_PROXY=base, all_proxy=base,
                             NO_PROXY='', no_proxy='')
            with session(options, proxy_env) as tty:
                tty.wait('No tests yet')
                tty.send('\t' * 5)
                tty.wait('DIAGNOSTIC WORKBENCH')
                tty.send('jjj\r')  # Stability monitor; ready screen is still offline.
                tty.wait('READY TO START')
                before = server.requests
                tty.send('\r')
                until = time.monotonic() + 5
                while server.requests == before and time.monotonic() < until:
                    tty.pump()
                assert server.requests > before, 'diagnostic child did not reach local proxy'
                tty.send('\x1b')
                tty.wait('CONFIRM CANCELLATION')
                tty.send('y')
                tty.wait('DIAGNOSTIC WORKBENCH')
                count = server.requests
                tty.pump(1.3)
                assert server.requests == count, 'cancelled diagnostic child kept probing'
                tty.send('q')
                tty.finish()
            transcript.append('PASS: cancelled Stability command stopped requests to the loopback-only HTTPS proxy.\n')

            # Real saved history, selection retention, comparison and corrupt-history recovery.
            root.mkdir(parents=True, exist_ok=True)
            records = []
            for i in range(25):
                item = dict(result)
                item['timestamp'] = (datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(minutes=25-i)).isoformat()
                item['backend'] = f'fixture-{i:02}'
                records.append(item)
            history = root / 'history.jsonl'
            history.write_text(''.join(json.dumps(item) + '\n' for item in records), encoding='utf-8')
            with session(options) as tty:
                tty.wait('LATEST RESULT')
                tty.send('\t\t')
                tty.wait('YOUR NETWORK, OVER TIME')
                tty.send('j' * 24)
                tty.wait('fixture-00')
                tty.send('\r')
                tty.wait('MEASUREMENT COMPLETE')
                tty.wait('fixture-00')
                tty.send('\x1b')
                tty.wait('YOUR NETWORK, OVER TIME')
                tty.send('c')
                tty.wait('BEFORE / AFTER')
                tty.send('\x1b')
                tty.send('\t')
                tty.wait('25 RUNS')
                tty.snapshot('Statistics from saved loopback fixtures')
                tty.send('q')
                tty.finish()
            history.write_text('not-json\n', encoding='utf-8')
            with session(options) as tty:
                tty.wait('HISTORY UNAVAILABLE')
                tty.send('\t\t')
                tty.wait('invalid history record')
                tty.send('r')
                tty.wait('invalid history record')
                tty.send('q')
                tty.finish()

            with session(options) as tty:
                tty.wait('HISTORY UNAVAILABLE')
                tty.send('\x03')
                tty.finish(130)
            with session(options) as tty:
                tty.wait('HISTORY UNAVAILABLE')
                tty.process.send_signal(signal.SIGINT)
                tty.finish(130)
            transcript.append('PASS: offline menu, 80x24 navigation, local diagnostic command and child cancellation, explicit --plain, resize guard, JSON/CSV export, persistence/no-save, cancellation, retry, history/compare/stats, Ctrl+C and SIGINT restoration.\nOnly loopback network traffic was used. No public probes or DNS changes ran.\n')
    finally:
        output.write_text('\n'.join(transcript), encoding='utf-8')
        server.shutdown()
        server.server_close()
    print(transcript[-1].strip())


if __name__ == '__main__':
    main()
