#!/usr/bin/env bash
# Package an already-built binary; never execute it. Requires Python 3 and Cargo
# unless an explicit metadata fixture is supplied as argument 4 (tests only).
# Usage: bash scripts/package-release.sh linux|darwin|windows BINARY OUTPUT_DIR [METADATA_JSON]
set -euo pipefail
if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo 'usage: package-release.sh linux|darwin|windows BINARY OUTPUT_DIR [METADATA_JSON]' >&2
  exit 2
fi
root=$(cd "$(dirname "$0")/.." && pwd)
platform=$1
binary=$2
output=$3
case "$platform" in linux|darwin|windows) ;; *) echo 'unsupported platform' >&2; exit 2 ;; esac
[[ -f "$binary" ]] || { echo 'missing binary' >&2; exit 2; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
if [[ $# == 4 ]]; then
  metadata=$4
else
  # The workflow passes the same target used for its build. Offline + locked
  # prevents packaging from changing resolution or downloading dependencies.
  # Use the caller's Cargo (CI selects 1.96.1); standalone installations need
  # not provide rustup shims merely to inspect locked package metadata.
  target=${CARGO_BUILD_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
  cargo metadata --manifest-path "$root/Cargo.toml" --offline --locked \
    --format-version 1 --filter-platform "$target" > "$work/metadata.json"
  metadata=$work/metadata.json
fi
python3 - "$root" "$platform" "$binary" "$output" "$metadata" "$work" <<'PY'
import base64
import hashlib
import json
import pathlib
import shutil
import sys
import tarfile
import zipfile

root, platform, binary, output, metadata, work = sys.argv[1:]
root, binary, output, metadata, work = map(pathlib.Path, [root, binary, output, metadata, work])
output.mkdir(parents=True, exist_ok=True)
stage = work / 'stage'
stage.mkdir()
name = 'bourso-cli.exe' if platform == 'windows' else 'bourso-cli'
shutil.copyfile(binary, stage / name)
(stage / name).chmod(0o755)
shutil.copyfile(root / 'LICENSE', stage / 'LICENSE')
data = json.loads(metadata.read_text())
members = set(data['workspace_members'])
notices = [
    'Third-party dependency inventory from locked Cargo metadata.',
    'Conservative target-filtered inventory: includes transitive, build and dev dependencies;',
    'not a claim that every listed crate is linked into the executable.',
    'License expressions and source URLs below are verbatim package metadata, not legal advice.',
    'Available upstream LICENSE/LICENCE/COPYING/NOTICE files are copied under third-party-licenses/.',
    'This does not certify all license obligations or cover system libraries/toolchains.',
    'Review distribution obligations, including source availability, before publication.', '',
]
for package in sorted(data['packages'], key=lambda p: (p['name'], p['version'])):
    if package['id'] in members:
        continue
    label = f"{package['name']}-{package['version']}"
    if pathlib.PurePath(label).name != label or '/' in label or '\\' in label:
        raise SystemExit('unsafe package name')
    base = pathlib.Path(package['manifest_path']).parent.resolve()
    # Include nested notices too (e.g. bundled native implementations), not just
    # the top-level SPDX declaration. Do not follow symlinks outside the crate.
    texts = {p for p in base.rglob('*') if p.is_file() and
             p.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE'))}
    if package.get('license_file'):
        texts.add(base / package['license_file'])
    if not texts:
        raise SystemExit(f'no upstream license/notice text found for {label}; legal review required')
    notices.extend([f"{package['name']} {package['version']}",
                    f"License: {package.get('license') or 'See declared license_file (no SPDX expression)'}",
                    f"Source: {package.get('source') or 'local path dependency'}"])
    for text in sorted(texts):
        resolved = text.resolve()
        if not resolved.is_relative_to(base) or not resolved.is_file():
            raise SystemExit(f'unsafe/missing license file for {label}')
        relative = text.relative_to(base)
        dest = stage / 'third-party-licenses' / label / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(text, dest)
        notices.append(f'Notice: third-party-licenses/{label}/{relative.as_posix()}')
    notices.append('')
(stage / 'THIRD_PARTY_NOTICES.txt').write_text('\n'.join(notices), encoding='utf-8')
suffix = '.zip' if platform == 'windows' else '.tar.gz'
archive = output / f'bourso-cli-{platform}{suffix}'
# Exclusive creation refuses to accidentally overwrite an existing release.
if platform == 'windows':
    with zipfile.ZipFile(archive, 'x', zipfile.ZIP_DEFLATED) as z:
        for p in sorted(stage.rglob('*')):
            if p.is_file():
                z.write(p, p.relative_to(stage).as_posix())
else:
    with tarfile.open(archive, 'x:gz') as t:
        for p in sorted(stage.rglob('*')):
            if p.is_file():
                t.add(p, arcname=p.relative_to(stage).as_posix(), recursive=False)
subject = f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n'
(output / 'SHA256SUMS').write_text(subject, encoding='utf-8')
(output / 'subjects.base64').write_text(base64.b64encode(subject.encode()).decode() + '\n', encoding='ascii')
print(archive)
PY
