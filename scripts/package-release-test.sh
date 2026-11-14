#!/usr/bin/env bash
# Hermetic archive tests: fixtures are data, never executed; no Cargo/network/bank.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/dependency"
printf 'FAKE BINARY - DO NOT EXECUTE\n' > "$work/binary"
printf 'Fixture dependency license text\n' > "$work/dependency/LICENSE-MIT"
python3 - "$work" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
m = {'workspace_members': ['local'], 'packages': [
    {'id': 'local', 'name': 'bourso-cli', 'version': '0.0.0', 'license': 'MIT'},
    {'id': 'fixture', 'name': 'fixture-dependency', 'version': '1.2.3',
     'license': 'MIT', 'license_file': None, 'source': 'registry+fixture',
     'manifest_path': str(p / 'dependency/Cargo.toml')}]}
(p / 'metadata.json').write_text(json.dumps(m))
PY
for platform in linux darwin windows; do
  bash "$root/scripts/package-release.sh" "$platform" "$work/binary" "$work/$platform" "$work/metadata.json"
done
python3 - "$work" "$root" <<'PY'
import base64, hashlib, io, pathlib, sys, tarfile, zipfile
work, root = map(pathlib.Path, sys.argv[1:])
for platform in ['linux', 'darwin', 'windows']:
    out = work / platform
    archive = next(out.glob('*.zip' if platform == 'windows' else '*.tar.gz'))
    if platform == 'windows':
        with zipfile.ZipFile(archive) as z:
            files = {n: z.read(n) for n in z.namelist() if not n.endswith('/')}
    else:
        with tarfile.open(archive) as t:
            files = {m.name: t.extractfile(m).read() for m in t.getmembers() if m.isfile()}
    binary = 'bourso-cli.exe' if platform == 'windows' else 'bourso-cli'
    assert files[binary] == (work / 'binary').read_bytes()
    assert files['LICENSE'] == (root / 'LICENSE').read_bytes()
    assert b'fixture-dependency 1.2.3' in files['THIRD_PARTY_NOTICES.txt']
    assert files['third-party-licenses/fixture-dependency-1.2.3/LICENSE-MIT'] == b'Fixture dependency license text\n'
    expected = f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n'
    assert (out / 'SHA256SUMS').read_text() == expected
    assert base64.b64decode((out / 'subjects.base64').read_text()).decode() == expected
print('PASS: Linux/macOS tar + Windows zip contents, project/third-party license, archive provenance')
PY
# Missing binary, unsupported OS and missing third-party texts must fail closed.
if bash "$root/scripts/package-release.sh" linux "$work/missing" "$work/bad" "$work/metadata.json"; then exit 1; fi
if bash "$root/scripts/package-release.sh" unknown "$work/binary" "$work/bad" "$work/metadata.json"; then exit 1; fi
rm "$work/dependency/LICENSE-MIT"
if bash "$root/scripts/package-release.sh" linux "$work/binary" "$work/bad" "$work/metadata.json"; then exit 1; fi
echo 'PASS: invalid inputs and absent license texts rejected'
