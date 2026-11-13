# .41 — reproduced capture defect and local workaround

2026-11-13. Beads owns status and criteria; this is verification evidence only.

## Reproduction before helper correction

Research157 identified a schedule distinct from the previous two-dispatcher
ON/OFF stress: a cold DEBUG callsite is first registered by an uncaptured thread
while exactly one captured ON dispatcher remains live. Locked tracing-core0.1.33
selects its `JustOne` path, consulting the registering thread's NoSubscriber,
which can cache Never despite the ON dispatcher and TRACE maximum elsewhere.

Worker159 executed a fresh-process harness and then an isolated child using the
actual unchanged helper. Both show root/resumed thread stable, TRACE maximum,
control events delivered and two target invocations counted, but captured phase2
zero versus expected one. The harness rebuild control delivers phase3. Logs and
small harness source are retained here. These are fresh actual red results before
any helper edit. They establish this mechanism, NOT attribution of Worker144's
historical missing monetary log. No historical execution ordering was recovered.

## Correction and exact candidate

Only `client/logging_tests.rs` changed. `capture` creates two DISTINCT registered
dispatchers and retains both through its action. The inert OFF instance is never
installed as thread/global subscriber and does not capture or raise the maximum.
Cloning one dispatcher would not register twice. Dependencies, production source,
monetary assertions and prior ON/OFF tests are unchanged. No general serialization.

Frozen candidate:
`c438cbc4bae8be42976ba2894f45c727aae8aba10dd11e18013471f19179f830`.
Inputs/criteria are in capture-fix-verification; before/after SAME, independent
reviewer's captures also SAME. Output-only directories are explicitly excluded.
Sources/tests/scripts/configuration/docs/legal texts actually consumed are inputs.
Preliminary red logs are not retroactively relabeled as frozen green evidence.
Source commit: `a73ab84df339b50fba0d56a4f4c8a762d6426da7`.

## Executed gates, Rust/Cargo1.96.1 explicit PATH

- Same real-helper reproduction now PASS, captured phase2 exactly once.
- Workspace tests207PASS:81CLI+126API. Three ignored helper entries are run by
  their parent tests; the new isolated child is explicitly exercised.
- Three API suites testthreads8 PASS; previous stress/money/redaction assertions
  retained. Finite runs are not universal flake-elimination proof.
- fmt/strict workspace all-target Clippy/locked offline tests PASS.
- DCA fake/packaging fixtures/coverage offline PASS.
- API coverage including test/support:97.38%lines,96.43%regions,94.30%functions.
  No threshold, CLI coverage, production transport or bank claim.

## Independent review receipt — reviewer20261113_162

Coordinator summary of returned judgment, not an invented reviewer-authored file.
Reviewer had no construction participation or write permission on the candidate;
received scope/factual context without requested verdict and inspected raw evidence,
source/diff and locked registry source against cached archive hashes. Incidental
historical builder notes exposed, not used as acceptance proof.

Verdict **accepted CAPTURE1/2/3**, local helper workaround only. Small-task
slice-acceptance1.1.0 scoreN/A. Replayed workspace207tests, cold child, stress,
two parallel API suites, fmt and strict Clippy; raw logs in reviewer-162. Existing
DCA/package/coverage logs inspected, not reviewer-rerun. No bank or network calls.

Limits: historical missing transfer-log attribution unknown; no universal capture
reliability or production tracing repair claim. Parent child `.output()` lacks
intrinsic deadline and arbitrary pre-barrier failures can hang; external test/job
timeouts only. Exact child name and execution verified today; future name changes
could select zero tests because parent currently checks status only. These were
nonblocking observations, not asserted fixed. Requested model gpt-6-astra is not
backend identity attestation.

## Outcome and remaining obligations

.41 closed after criteria evidenced and independent judgment. Total32/41 children
closed; original nine obligations .4/.6/.7/.8/.24/.26/.29/.33/.34 and epic remain
open. Supplier contract, direct-library durable recovery decision, remote/native
CI and FORMAT-2 historical gaps unchanged, without artificial closure.

No dependency upgrade, real secret, bank operation, push, tag, publication or deploy.
Next action: obtain existing anonymized supplier evidence or explicit decisions
listed in docs/FINANCIAL_CONTRACTS.md. Do not infer bank readiness from test repair.
