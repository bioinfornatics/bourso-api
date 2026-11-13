#!/usr/bin/env python3
"""Snapshot exactly the legal inputs consumed by package-release.sh (offline).
Run from repository root after recording cargo metadata as metadata.original.json.
Does not copy crate sources or invent upstream texts. Refuses an existing snapshot.
"""
import hashlib
import json
from pathlib import Path
import shutil

root = Path.cwd()
out = root / 'docs/delivery/legal-inputs'
metadata = json.loads((out / 'metadata.original.json').read_text())
members = set(metadata['workspace_members'])
records = []
for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
    if package['id'] in members:
        continue
    label = f"{package['name']}-{package['version']}"
    assert Path(label).name == label and '\\' not in label
    base = Path(package['manifest_path']).parent.resolve()
    texts = {p for p in base.rglob('*') if p.is_file() and
             p.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE'))}
    if package.get('license_file'):
        texts.add(base / package['license_file'])
    assert texts, label
    destbase = out / 'crates' / label
    for text in sorted(texts):
        resolved = text.resolve()
        assert resolved.is_relative_to(base) and resolved.is_file()
        relative = text.relative_to(base)
        dest = destbase / relative
        assert not dest.exists(), dest
        dest.parent.mkdir(parents=True, exist_ok=True)
        before = text.read_bytes()
        dest.write_bytes(before)
        assert text.read_bytes() == dest.read_bytes() == before
        records.append({'package_id': package['id'], 'package': label,
                        'source_path': str(text),
                        'snapshot_path': dest.relative_to(root).as_posix(),
                        'archive_path': f'third-party-licenses/{label}/{relative.as_posix()}',
                        'size': len(before), 'sha256': hashlib.sha256(before).hexdigest()})
    # Only consumed path fields are relocated. All package IDs, SPDX expressions,
    # source URLs, dependency resolution and original metadata are preserved.
    package['manifest_path'] = (destbase / 'Cargo.toml').relative_to(root).as_posix()
(out / 'metadata.snapshot.json').write_text(json.dumps(metadata, indent=2) + '\n')
(out / 'legal-files.json').write_text(json.dumps(records, indent=2) + '\n')
(out / 'SHA256SUMS').write_text(''.join(f"{r['sha256']}  {r['snapshot_path']}\n" for r in records))
print(f'{len(records)} exact legal files; {len(set(r["package"] for r in records))} third-party packages; {sum(r["size"] for r in records)} bytes')
