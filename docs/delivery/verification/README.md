# Local legal-input and format verification (worker 20261113_96)

Evidence support for Beads `bourso-y3u.32` / `.33`; **not a status tracker**.
Canonical acceptance is the read-only ticket snapshot in
`../legal-inputs/criteria-readonly.json` (including the recorded review-91 limits).
The full standalone review-91 report was not found; its consolidation in those
notes is the available source. No criterion is changed by this evidence.

## Scope and limitations fixed before checks

- Preserve the existing dirty working tree. This worker writes only `docs/delivery/**`.
  `scripts/package-release*.sh`, business Rust, manifests and lockfiles stay unchanged.
- LICENSE-1: create all three archive container formats with a **non-executable
  binary fixture**, real target-filtered locked metadata, and exact upstream texts.
  This tests local packaging, not native macOS/Windows binaries or completeness of
  distribution obligations on other targets. Inventory target: x86_64-unknown-linux-gnu.
- LICENSE-2: verify both workspace packages declare MIT, root LICENSE is MIT, and
  relocation changes only third-party `manifest_path`, never license/source values.
- LICENSE-3: inspect every archive byte inventory and SHA256/base64 subject mapping.
  This is not a signed SLSA attestation or publication. README's verifier example
  names the distribution archive and explicitly disclaims an executed release check.
- FORMAT-1: run `cargo fmt --all -- --check` twice with Rust 1.96.1.
- FORMAT-2: compare the entire captured candidate before/after these read-only
  checks. Equality proves **only this current no-op**, not historical formatting.
  `/tmp/bourso-fmt-before.log` contains ANSI rustfmt diff fragments, not an isolated,
  authenticated complete before-tree. It is not used to invent/reconstruct one.
  A Rust lexer would not recover missing inputs; no token-equivalence claim is made.
- FORMAT-3: not rerun here. Prior test reports are historical, not promoted to this
  candidate. Missing historical baseline remains explicit; no task closure.
- This is worker-local input stability evidence, **not a coordinator global freeze**.
  No other writer is expected per delegation; capture is not an atomic snapshot.
  The coordinator must freeze final scope and independent review must check coverage.

## Inputs and provenance

`legal-inputs/metadata.original.json` was obtained offline with the installed pinned
Cargo: `cargo metadata --offline --locked --format-version 1 --filter-platform
x86_64-unknown-linux-gnu`. `snapshot_legal.py` copies only files selected by the
packager's current LICENSE/LICENCE/COPYING/NOTICE scan plus declared license_file.
`legal-files.json` maps every original absolute cache path and package ID to a
root-relative preserved path, archive path, size and SHA-256. No crate sources are
copied. Original metadata is retained. `metadata.snapshot.json` relocates only
third-party manifest paths into that snapshot (Cargo.toml itself need not exist:
packager consumes its parent directory, not its content).

The packager's existing fourth positional argument supplies this metadata; no new
option or script edit was needed. Run from repository root, because relocated paths
are root-relative. The current test input is explicitly `binary.fixture`, not a
previously built executable with unknown build provenance.

## Replay (no network, install, bank, tracker mutation or Git commit)

From repository root, select the already-installed toolchain, then a **new output
path outside captured inputs** (or under the declared output-only evidence path):

```sh
export PATH=/home/jmercier/.rustup/toolchains/1.96.1-x86_64-unknown-linux-gnu/bin:$PATH
export CARGO_NET_OFFLINE=true
python3 -B docs/delivery/verification/scripts/candidate_identity.py capture \
  --root "$PWD" --scope docs/delivery/verification/scope.json > docs/delivery/evidence-96/before.json
python3 -B docs/delivery/verification/scripts/verify_local.py docs/delivery/evidence-96/run
python3 -B docs/delivery/verification/scripts/candidate_identity.py capture \
  --root "$PWD" --scope docs/delivery/verification/scope.json > docs/delivery/evidence-96/after.json
python3 -B docs/delivery/verification/scripts/candidate_identity.py compare \
  docs/delivery/evidence-96/before.json docs/delivery/evidence-96/after.json
(cd docs/delivery/evidence-96/run && sha256sum --check ARCHIVES.sha256)
```

Do not overwrite the preserved first run; choose fresh manifest/output filenames
for another run. The archive timestamps are intentionally not normalized by the
existing packager: replay is content/check reproducibility, **not byte-identical
archive builds**. Each produced archive has its own checked hashes. Toolchain
binaries are externally installed; observed versions and binary hashes are recorded,
not a vendored/hermetic compiler closure. The copied identity verifier and schema,
verification scripts, scope, root LICENSE, docs and legal inputs are captured.
Only `docs/delivery/evidence-96` is output-only and excluded; never consume its
outputs as candidate inputs under that exclusion.
