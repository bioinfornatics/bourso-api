# Evidence handoff — worker 20261113_96

Generated UTC: 2026-11-13T17:24:21.750778+00:00

## Candidate and replay

Worker-local candidate: `d2f8dafc911e848025348f45b7af1981c455e8d864fbf713d39f5540b65aeb59`; 403 entries.
`final-before.json` and `final-after.json`: validated; comparison `SAME`.
This is not a coordinator global freeze or an independent acceptance verdict.
Inputs: all source/test/script/config/docs paths in `../verification/scope.json`,
including root LICENSE and 313 preserved legal texts. Only this output directory
is excluded. Replay instructions: `../verification/README.md`.
Final command/exit evidence: `final-run/commands.json`; inspection:
`final-run/inspection.json`; archive hashes: `final-run/ARCHIVES.sha256`.
Preliminary run also retained; final rerun adds instruction snapshots to identity.

## Criterion evidence (canonical truth remains in Beads)

- LICENSE-1: PASS local fixture archive-container inspection on Linux/macOS/Windows
  formats, 313 exact third-party legal texts from 181 packages plus root LICENSE
  in every archive. Real Linux-target offline locked dependency metadata; fake
  non-executable binary. No claim about native cross-platform binaries or complete
  legal obligations. Source availability/system libraries/toolchains remain outside
  the packager's stated inventory.
- LICENSE-2: PASS two MIT workspace metadata declarations and MIT root LICENSE;
  snapshot relocation preserves all metadata except third-party manifest paths.
  No upstream license text or SPDX value invented.
- LICENSE-3: PASS archive checksum and base64 subject association for all outputs;
  README example addresses the archive, not extracted executable. Signed SLSA
  verification NOT RUN (no released artifact or signed attestation).
- FORMAT-1: PASS twice `cargo fmt --all -- --check`, pinned cargo/rustc 1.96.1,
  rustfmt 1.9.0-stable. Empty outputs and exit 0 retained.
- FORMAT-2: current checks are a no-op; exact before/after candidate equality.
  Historical isolated formatting diff/baseline UNAVAILABLE; not certified.
  The old `/tmp/bourso-fmt-before.log` contains fragments and is not used as a
  reconstructed whole-tree proof. No lexer/token equivalence claim.
- FORMAT-3: NOT RUN here. Historical passing tests are not rebound to this candidate.
  Worker stops the formatting slice rather than weakening this missing gate.

## Files, constraints and limitations

Added only `docs/delivery/**` (about 6.8 MiB including both preserved archive runs).
313 raw legal texts total 1,535,928 bytes. No full package source trees copied.
Existing metadata fixture argument permits replay from the snapshot; no production
script change necessary. Scripts retain SHA-256:
- package-release.sh: c36a23d19a7d35d49d7449cebeb3980d83c3b7c45f689bf52bbb1e29299878eb
- package-release-test.sh: 92028bd4b09c5c56793f00d75b5c7b6c13636b8076336c5fd67a14348678f2e1

Existing tracked/untracked work preserved; branch main unchanged. No business
source/Cargo modification, network, install, commit, push, merge, bank invocation
or tracker mutation performed. Beads .32/.33 read back blocked before work; no
closure attempted. Reviewer 91's consolidation was read from their ticket notes;
no standalone full review-91 report found. Tool binaries remain installed external
runtime: version and hashes recorded, not full hermetic toolchain closure.
Archive timestamps are not normalized: reproducible contents/checks and per-run
hash association, not a claim of identical archive hashes on every rerun.
Judge not run; score not assessed. Independent review and global freeze belong to
coordinator, not this worker. No native platform/release readiness established.

## Restart prompt

Resume bourso-y3u.32/.33 in /home/jmercier/Codes/me/bourso-api, main, with existing
uncommitted work preserved. Read current Beads criteria read-only and
`docs/delivery/verification/README.md`, `scope.json`, this evidence and
`final-before.json`. Revalidate permissions, candidate freshness and single-writer
ownership. First freeze the final coordinator candidate before independent review;
replay `verify_local.py` with a fresh output directory under declared output-only
scope, compare before/after identity and inspect preserved archives. Do not promote
worker fixture proof to native release or legal certification. FORMAT-2 historical
baseline and FORMAT-3 comparative test evidence are missing: obtain authenticated
inputs or leave the task open, never reinterpret away its criteria. No bank,
network, installation, Git commit or tracker update authority is conveyed by this
handoff. Suggested next-session name: legal packaging — independent local review.
