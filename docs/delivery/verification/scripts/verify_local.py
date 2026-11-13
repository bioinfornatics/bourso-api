#!/usr/bin/env python3
"""Offline, fixture-only archive inspection and non-mutating formatting checks.
Run from repository root. Output directory must be new; no business source writes.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import zipfile

root = Path.cwd()
inputs = root / 'docs/delivery/legal-inputs'
out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=False)
results = []

def run(label, args):
    p = subprocess.run(args, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (out / f'{label}.log').write_bytes(p.stdout)
    results.append({'check': label, 'command': args, 'exit': p.returncode})
    (out / 'commands.json').write_text(json.dumps(results, indent=2) + '\n')
    assert p.returncode == 0, label

records = json.loads((inputs / 'legal-files.json').read_text())
expected = {}
for r in records:
    text = (root / r['snapshot_path']).read_bytes()
    assert hashlib.sha256(text).hexdigest() == r['sha256']
    assert len(text) == r['size']
    assert r['archive_path'] not in expected
    expected[r['archive_path']] = text
original = json.loads((inputs / 'metadata.original.json').read_text())
snapshot = json.loads((inputs / 'metadata.snapshot.json').read_text())
# Verify relocation did not alter any legal metadata or dependency information.
for old, new in zip(original['packages'], snapshot['packages'], strict=True):
    new = dict(new)
    new['manifest_path'] = old['manifest_path']
    assert old == new
assert {k:v for k,v in original.items() if k != 'packages'} == {k:v for k,v in snapshot.items() if k != 'packages'}
members = set(snapshot['workspace_members'])
workspace = [p for p in snapshot['packages'] if p['id'] in members]
assert len(workspace) == 2 and all(p['license'] == 'MIT' for p in workspace)
assert b'MIT License' in (root / 'LICENSE').read_bytes()
run('existing-package-tests', ['bash', 'scripts/package-release-test.sh'])
archive_sums = []
for platform in ['linux', 'darwin', 'windows']:
    run(f'package-{platform}', ['bash', 'scripts/package-release.sh', platform,
        str(inputs / 'binary.fixture'), str(out / platform), str(inputs / 'metadata.snapshot.json')])
    archive = out / platform / f'bourso-cli-{platform}{".zip" if platform == "windows" else ".tar.gz"}'
    if platform == 'windows':
        with zipfile.ZipFile(archive) as z:
            names = z.namelist()
            assert len(names) == len(set(names))
            files = {n: z.read(n) for n in names}
    else:
        with tarfile.open(archive) as t:
            members = t.getmembers()
            assert all(m.isfile() for m in members)
            assert len(members) == len({m.name for m in members})
            files = {m.name: t.extractfile(m).read() for m in members}
    binary = 'bourso-cli.exe' if platform == 'windows' else 'bourso-cli'
    assert set(files) == set(expected) | {binary, 'LICENSE', 'THIRD_PARTY_NOTICES.txt'}
    assert files[binary] == (inputs / 'binary.fixture').read_bytes()
    assert files['LICENSE'] == (root / 'LICENSE').read_bytes()
    assert {n:b for n,b in files.items() if n.startswith('third-party-licenses/')} == expected
    notice = files['THIRD_PARTY_NOTICES.txt'].decode()
    assert {line.removeprefix('Notice: ') for line in notice.splitlines() if line.startswith('Notice: ')} == set(expected)
    for p in snapshot['packages']:
        if p['id'] not in snapshot['workspace_members']:
            assert f"{p['name']} {p['version']}\nLicense: {p.get('license') or 'See declared license_file (no SPDX expression)'}\nSource: {p.get('source') or 'local path dependency'}\n" in notice
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    subject = f'{digest}  {archive.name}\n'
    assert (archive.parent / 'SHA256SUMS').read_text() == subject
    assert base64.b64decode((archive.parent / 'subjects.base64').read_bytes()).decode() == subject
    archive_sums.append(f'{digest}  {platform}/{archive.name}\n')
    (archive.parent / 'THIRD_PARTY_NOTICES.txt').write_bytes(files['THIRD_PARTY_NOTICES.txt'])
(out / 'ARCHIVES.sha256').write_text(''.join(archive_sums))
run('fmt-before', ['cargo', 'fmt', '--all', '--', '--check'])
# Repeat the read-only command, never rewrite protected source files.
run('fmt-after', ['cargo', 'fmt', '--all', '--', '--check'])
(out / 'inspection.json').write_text(json.dumps({
    'legal_files': len(expected), 'third_party_packages': len(snapshot['packages']) - len(workspace),
    'platform_containers': ['linux', 'darwin', 'windows'],
    'binary': 'non-executable fixture; no native build or binary execution',
    'checks': 'archive bytes, exact legal inventory, verbatim SPDX/source, project MIT, checksum and base64 association passed',
    'format': 'two read-only workspace checks passed; before/after candidate identity must be compared separately',
    'historical_format_delta': 'unavailable; no historical semantic-equivalence claim',
}, indent=2) + '\n')
print(f'PASS: 3 fixture archive containers, {len(expected)} exact texts each; metadata/provenance; rustfmt twice')
