#!/usr/bin/env python3
"""Eight-language real-terminal checks; only loopback, no public probes or DNS writes."""
import argparse
import contextlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

from cockpit_smoke import CountedFixture, Tty


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/debug/speedtest.exe' if os.name == 'nt' else 'target/debug/speedtest')
    parser.add_argument('--output', default='verification/localization-smoke.txt')
    args = parser.parse_args()
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    if os.name == 'nt':
        output.write_text('SKIPPED: Unix PTY only; eight-language Rust rendering and executable contracts run on Windows.\n', encoding='utf-8')
        print(output.read_text().strip())
        return
    root = Path(__file__).resolve().parents[2]
    binary = str(Path(args.binary).resolve(strict=True))
    transcript = []
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), CountedFixture)
    server.daemon_threads = True
    server.requests = 0
    server.count_lock = threading.Lock()
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f'http://127.0.0.1:{server.server_port}'
    options = ['--backend', 'librespeed', '--librespeed-server', base,
               '--duration', '3', '--streams', '1', '--timeout', '20', '--no-save']
    try:
        with tempfile.TemporaryDirectory(prefix='speedtest-locales-') as scratch:
            env = {k: v for k, v in os.environ.items() if k not in ('NO_COLOR', 'CLICOLOR', 'CLICOLOR_FORCE')}
            env.update(HOME=scratch, XDG_DATA_HOME=scratch, LOCALAPPDATA=scratch,
                       LC_ALL='C', SPEEDTEST_LANGUAGE='en', TERM='xterm-256color',
                       COLORTERM='truecolor', NO_PROXY='*', no_proxy='*', RUST_BACKTRACE='0')

            @contextlib.contextmanager
            def session(arguments):
                tty = Tty(binary, arguments, env, transcript)
                try:
                    yield tty
                except BaseException:
                    tty.snapshot('FAILURE')
                    raise
                finally:
                    tty.close()

            def translator(code):
                catalog = json.loads((root / 'src/i18n/locales' / f'{code}.json').read_text(encoding='utf-8'))
                def tr(key):
                    if code == 'en':
                        return key
                    value = catalog[key.strip().lower()]
                    return value.upper() if key.isupper() else value
                return tr

            for code in ('en', 'it', 'es', 'fr', 'de', 'pt', 'zh-CN', 'ja'):
                tr = translator(code)
                with session([*options, '--language', code]) as tty:
                    tty.wait(tr('No tests yet'))
                    assert 'LAST RESULT AVAILABLE' not in tty.screen.text()
                    assert server.requests == 0, 'translated startup made requests'
                    tty.snapshot(f'{code} / 80x24 / home')
                    tty.send('z')
                    tty.wait(tr('TEXT SIZE'))
                    tty.send('\x1b[6~')  # PageDown in a translated, scrollable modal.
                    tty.snapshot(f'{code} / font-size help (scrolled)')
                    tty.send('\x1b')
                    tty.send('\t' * 6)
                    tty.wait(tr('MAKE IT YOURS'))
                    tty.send('j' * 10)
                    tty.wait(tr('Language'))
                    tty.send('\r')
                    # Changes language in-place; Settings remains selected.
                    tty.snapshot(f'{code} / immediate language change')
                    tty.resize(120, 38)
                    tty.send('z')
                    tty.send('\x1b')
                    tty.resize(80, 24)
                    tty.send('q')
                    tty.finish()
            assert server.requests == 0, 'menu/settings/size help made requests'

            # Reuse the existing command implementation, in the selected locale.
            tr = translator('zh-CN')
            with session([*options, '--language', 'zh-CN']) as tty:
                tty.wait(tr('No tests yet'))
                tty.send('\t' * 4 + 'j\r')  # DNS -> local resolver catalog.
                tty.wait(tr('READY TO START'))
                tty.send('\r')
                tty.wait('DNS')
                tty.wait('cloudflare')
                tty.snapshot('Chinese DNS catalog from the existing CLI child')
                tty.send('q')
                tty.finish()
            assert server.requests == 0, 'local DNS catalog made requests'

            # The immediate legacy gauge also honors the selected language.
            tr = translator('it')
            with session([*options, '--language', 'it', '--run']) as tty:
                tty.wait(tr('LATENCY'))
                tty.snapshot('Italian --run keeps the original gauge and translated controls')
                tty.send('\x03')
                tty.finish(130)

            # A real localized measurement exports the same canonical model.
            path = Path(scratch) / 'italian.json'
            tr = translator('it')
            with session([*options, '--language', 'it', '--output', str(path)]) as tty:
                tty.wait(tr('No tests yet'))
                tty.send('\r\r')
                tty.wait(tr('MEASUREMENT COMPLETE'), timeout=25)
                tty.wait(tr('EXPORTED • automatic history is off').split(' • ')[0])
                tty.snapshot('Italian completion and unchanged canonical export')
                record = json.loads(path.read_text(encoding='utf-8'))
                assert record['backend'] == 'librespeed'
                assert record['download']['bytes'] > 0 and record['upload']['bytes'] > 0
                assert not list(Path(scratch).rglob('history.jsonl')), '--no-save saved history'
                tty.send('q')
                tty.finish()
            # Locale must not alter the offline JSON report for that exact result.
            reports = []
            for code in ('en', 'it', 'ja'):
                result = subprocess.run([binary, 'check', str(path), '--min-download', '0', '--json', '--language', code],
                                        env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=5)
                assert result.returncode == 0 and not result.stderr
                reports.append(result.stdout)
            assert reports[0] == reports[1] == reports[2]
        transcript.append('PASS: eight languages, offline startup, badge removal, Settings language selection, font help scrolling, resize, localized diagnostic child and real loopback measurement, unchanged JSON, no-save and terminal restoration.\nNo public speed tests or DNS changes ran.\n')
        print(transcript[-1])
    finally:
        server.shutdown()
        server.server_close()
        output.write_text('\n'.join(transcript), encoding='utf-8')


if __name__ == '__main__':
    main()
