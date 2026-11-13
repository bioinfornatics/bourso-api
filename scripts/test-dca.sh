#!/usr/bin/env bash
# Local fake executors only; never invokes the bank CLI or loads real credentials.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
python3 - "$root/bourso-launchd.sh" <<'PY'
import os, pathlib, signal, stat, subprocess, sys, tempfile, time
launcher = sys.argv[1]
with tempfile.TemporaryDirectory(prefix='dca-fixtures-') as tmp:
    base = pathlib.Path(tmp)
    fake = base / 'fake executor'
    fake.write_text('''#!/bin/bash
printf '%s\\n' "$*" >> "$CALLS"
[ -f "$DCA_STATE_DIR/attempt" ] || exit 88
if [ "${FAKE_MODE:-}" = crash ]; then kill -KILL "$PPID"; exit 23; fi
if [ "${FAKE_MODE:-}" = wait ]; then
  touch "$READY"
  while [ ! -f "$RELEASE" ]; do sleep 0.05; done
fi
exit "${FAKE_EXIT:-0}"
''')
    fake.chmod(0o700)
    # Even the obsolete launcher cannot open Terminal during the red regression run.
    shim = base / 'osascript'
    shim.write_text('#!/bin/bash\nexit 99\n')
    shim.chmod(0o700)
    credentials = base / 'existing-credentials.json'
    credentials.write_text('{}') # No actual credentials; fake ignores this path.
    credentials.chmod(0o600)
    env = dict(os.environ, PATH=str(base)+os.pathsep+os.environ['PATH'], HOME=str(base), SCRIPT_PATH=str(fake), SYMBOL='fixture-symbol',
               ACCOUNT='fixture-account', QUANTITY='1', CREDENTIALS_FILE=str(credentials),
               CALLS=str(base/'calls'), READY=str(base/'ready'), RELEASE=str(base/'release'))
    def scenario(name):
        e = dict(env, DCA_STATE_DIR=str(base/name))
        return e, pathlib.Path(e['DCA_STATE_DIR'])
    def run(e, *args, expected=None):
        p = subprocess.run(['bash', launcher, *args], env=e, stdin=subprocess.DEVNULL,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=10)
        if expected is not None:
            assert p.returncode == expected, (args, p.returncode, p.stdout, p.stderr)
        return p
    def calls():
        return (base/'calls').read_text().splitlines() if (base/'calls').exists() else []
    e, state = scenario('first')
    run(e, '--mode', 'existing-credentials', expected=3)
    assert len(calls()) == 1, 'configured executor must actually run'
    assert 'state=submitted-unverified' in (state/'attempt').read_text()
    assert 'exit_status=0' in (state/'attempt').read_text()
    assert not (state/'last_operator_confirmed').exists()
    assert stat.S_IMODE(state.stat().st_mode) == 0o700
    assert stat.S_IMODE((state/'attempt').stat().st_mode) == 0o600
    assert not (state/'lock').exists()
    run(e, '--mode', 'existing-credentials', expected=3)
    assert len(calls()) == 1
    print('PASS: initial state, direct invocation, private journal, zero exit unverified, repeat blocked')
    def attempt_id(state):
        return next(x[3:] for x in (state/'attempt').read_text().splitlines() if x.startswith('id='))
    ident = attempt_id(state)
    before = (state/'attempt').read_bytes()
    run(e, '--acknowledge-reviewed-attempt', 'wrong-id', '--outcome', 'not-applied', expected=2)
    run(e, '--acknowledge-reviewed-attempt', ident, '--outcome', 'confirmed', expected=2)
    assert (state/'attempt').read_bytes() == before
    run(e, '--acknowledge-reviewed-attempt', ident, '--outcome', 'confirmed',
        '--receipt-reference', 'fake-local-receipt', expected=0)
    assert len(calls()) == 1
    assert stat.S_IMODE((state/'last_operator_confirmed').stat().st_mode) == 0o600
    run(e, '--mode', 'existing-credentials', expected=0)
    assert len(calls()) == 1
    # Exercise the elapsed interval using synthetic state, never bank data.
    (state/'last_operator_confirmed').write_text(str(int(time.time()) - 604801)+'\n')
    run(e, '--mode', 'existing-credentials', expected=3)
    assert len(calls()) == 2
    assert len(list(state.glob('reviewed.*'))) == 1
    run(e, '--acknowledge-reviewed-attempt', attempt_id(state), '--outcome', 'not-applied', expected=0)
    run(e, '--mode', 'existing-credentials', expected=3)
    assert len(calls()) == 3
    print('PASS: explicit matched review, receipt required, no execution on review, weekly interval, reviewed history')

    e, state = scenario('failed')
    e['FAKE_EXIT'] = '17'
    run(e, '--mode', 'existing-credentials', expected=3)
    assert 'state=unknown' in (state/'attempt').read_text()
    assert 'exit_status=17' in (state/'attempt').read_text()
    count = len(calls())
    run(e, '--mode', 'existing-credentials', expected=3)
    assert len(calls()) == count
    assert not (state/'last_operator_confirmed').exists()
    print('PASS: nonzero exit stays unknown, blocks repeat, never writes confirmation')

    e, state = scenario('parallel')
    e['FAKE_MODE'] = 'wait'
    p = subprocess.Popen(['bash', launcher, '--mode', 'existing-credentials'], env=e,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 5
        while not (base/'ready').exists():
            assert p.poll() is None and time.monotonic() < deadline, 'fixture did not start'
            time.sleep(.02)
        assert (state/'lock').is_dir() and 'state=unknown' in (state/'attempt').read_text()
        assert stat.S_IMODE((state/'lock').stat().st_mode) == 0o700
        count = len(calls())
        run(e, '--mode', 'existing-credentials', expected=3)
        assert len(calls()) == count
    finally:
        (base/'release').touch()
        p.wait(timeout=10)
    assert p.returncode == 3 and not (state/'lock').exists()
    print('PASS: foreground child observed, concurrent invocation blocked, ordinary lock release')

    e, state = scenario('crash')
    e['FAKE_MODE'] = 'crash'
    p = run(e, '--mode', 'existing-credentials')
    assert p.returncode == -signal.SIGKILL
    assert (state/'lock').exists()
    assert 'state=unknown' in (state/'attempt').read_text()
    assert 'exit_status=not-observed' in (state/'attempt').read_text()
    count = len(calls())
    run(e, '--mode', 'existing-credentials', expected=3)
    assert len(calls()) == count and (state/'lock').exists()
    (state/'lock').rmdir() # Simulated explicit operator recovery, only in fixture.
    run(e, '--mode', 'existing-credentials', expected=3)
    assert len(calls()) == count
    print('PASS: killed supervisor leaves lock and unknown journal; no stale auto-delete or retry')

    e, state = scenario('tty')
    run(e, expected=2)
    assert not (state/'attempt').exists()
    assert len(calls()) == count
    e, state = scenario('placeholder')
    del e['SCRIPT_PATH']
    run(e, '--mode', 'existing-credentials', expected=2)
    assert not (state/'attempt').exists()
    e, state = scenario('public-state')
    state.mkdir(mode=0o755)
    run(e, '--mode', 'existing-credentials', expected=2)
    assert stat.S_IMODE(state.stat().st_mode) == 0o755
    e, state = scenario('linked-state')
    state.symlink_to(base/'first', target_is_directory=True)
    run(e, '--mode', 'existing-credentials', expected=2)
    e, state = scenario('malformed')
    state.mkdir(mode=0o700)
    (state/'attempt').write_text('garbage\n')
    (state/'attempt').chmod(0o600)
    run(e, '--mode', 'existing-credentials', expected=2)
    assert (state/'attempt').read_text() == 'garbage\n'
    assert len(calls()) == count
    print('PASS: no-TTY, placeholder, unsafe permissions, symlink and malformed state rejected without execution')

    import pty
    e, state = scenario('interactive')
    del e['CREDENTIALS_FILE']
    master, slave = pty.openpty()
    try:
        p = subprocess.run(['bash', launcher], env=e, stdin=slave,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        assert p.returncode == 3, (p.returncode, p.stderr)
    finally:
        os.close(slave)
        os.close(master)
    assert calls()[-1] == 'trade order new --side buy --symbol fixture-symbol --account fixture-account --quantity 1'
    assert 'state=submitted-unverified' in (state/'attempt').read_text()
    count = len(calls())
    e, state = scenario('clock')
    state.mkdir(mode=0o700)
    timestamp = state/'last_operator_confirmed'
    timestamp.write_text(str(int(time.time())+3600)+'\n')
    timestamp.chmod(0o600)
    run(e, '--mode', 'existing-credentials', expected=3)
    timestamp.write_text('not-a-time\n')
    run(e, '--mode', 'existing-credentials', expected=2)
    run(e, '--acknowledge-reviewed-attempt', '', expected=2)
    assert len(calls()) == count
    print('PASS: configured default mode invokes fake through TTY; future/corrupt timestamps and empty review rejected')
print('PASS: all DCA tests used isolated fake executors; no bank requests')
PY
