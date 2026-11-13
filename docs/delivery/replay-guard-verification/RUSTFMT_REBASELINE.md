# Rustfmt reference decision — bourso-y3u.33

## Authority and historical gap

On 2026-11-14 the user answered “procede” after the coordinator explained and
proposed accepting the historical evidence gap and using the verified current
code as a new reference, without rewriting Git history.

Historical FORMAT-2 (format-only diff of the earlier operation) and historical
FORMAT-3 (comparable before/after test results for that operation) remain
**NOT PROVEN**. The earlier mixed commit and rustfmt fragments do not establish
these properties. Three subsequent searches (reflogs, unreachable Git objects,
existing distribution archives) found no authenticated isolated source pair.
The original criteria remain recorded in Beads; this decision is not evidence
that the historical operation was pure formatting.

Risk accepted: inability to audit that earlier formatting step in isolation.
No claim of provider compatibility, financial acceptance, remote CI readiness or
absence of defects follows. No Git rebase, force push or history rewrite is
permitted by this decision.

## Applicable replacement obligation

- RB-1: preserve the gap, provenance, accepted risk and scoped user authority.
- RB-2: capture current code/tests/docs/config/scripts/legal inputs in an exact
  manifest, run Rust 1.96.1 fmt check and workspace tests, and compare before/after
  inputs. No new format-only commit is fabricated to represent the earlier step.
- RB-3: independent judge reviews application of this decision and current
  evidence. Any closure of .33 cites the decision rather than asserting
  historical FORMAT-2/3 PASS. Global .34 retains all other mandatory obligations.

## Reference and reproducibility

Use `scope.json`, the contained `scripts/candidate_identity.py` and its schema,
plus the rebaseline manifests/logs under the output-only
`../replay-guard-evidence/rebaseline/` directory. The manifest includes untracked
source inputs and complete Git index metadata; logs/caches remain output-only.
Bind the outcome to the candidate_id emitted by capture, not solely to HEAD.
The content reference can be checked again after a commit, but a changed HEAD or
index yields a new candidate identity and does not retroactively relabel tests.

Commands from the repository root, with the installed Rust 1.96.1 bin directory
first on PATH and no dependency download:

```sh
python3 -B docs/delivery/replay-guard-verification/scripts/candidate_identity.py capture \
  --root "$PWD" --scope docs/delivery/replay-guard-verification/scope.json
cargo fmt --all -- --check
cargo test --workspace --locked --offline
```

Capture again with the same scope and compare manifests. SAME means unchanged
inputs, not acceptance. Independent judgment and Beads readback remain required.
