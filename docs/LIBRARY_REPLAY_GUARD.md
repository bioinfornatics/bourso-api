# Opt-in library replay barrier — local, conservative

`bourso_api::replay_guard::ReplayGuard` is an additive orchestration helper for
order placement and immediate transfers. It is not bank idempotency, receipt
validation, automatic reconciliation or verified execution. Raw `order`,
`order_detailed`, `transfer_funds` and `transfer_money` APIs remain available and
bypass it. Adoption by direct library consumers is explicit; it does not impose
a global journal or make unguarded callers safe.

## Storage and coordination domain

Supply an **absolute path** to a trusted local Unix directory. Parent directories
must already exist; symlink ancestors and `..` components are refused. The helper
creates/tightens the directory to 0700 and regular files to 0600, rejects symlinks
and multiply-linked files, and does not reveal paths in its diagnostics. This is
not protection against malicious same-user filesystem replacement. Non-Unix
storage is refused; Windows ACL safety is not established.

All cooperating financial calls must use the same directory. A reserved order
blocks a transfer in that domain and vice versa. Changing the directory, deleting
files, using another machine or calling a raw API bypasses the barrier. CLI and
DCA journals remain separate and require separate operator review; no migration
or shared coordination with them is implied.

- `replay.lock`: stable regular file. An exclusive nonblocking OS lock is held
  for the guard handle's lifetime, including invocation and review. Never remove
  or replace it while any cooperating process can use the domain. Drop explicitly
  unlocks it but deletes no file.
- `replay-unresolved.json`: schema version, operation kind and a random local
  attempt identifier only. No account, amount, reason, bank reference, token,
  response or credentials are stored. The local ID is not an idempotency key.

Small blocking filesystem operations run on the calling thread. A hung call or
idle guard retains its lock until the handle is dropped/process terminated. There
is no whole-operation timeout, TTL or automatic stale-lock file deletion.

## Invocation and original results

`invoke` reserves the journal by exclusive creation, complete write and flush
**before calling its closure**, when the outer future is first polled. It retains
the journal after every result, error, panic, cancellation/drop or process crash.
A never-polled outer future neither reserves nor invokes. A failed reservation
never calls the closure; partial/unreadable/invalid journals remain blocking.

Construct the bank future inside the closure. For transfers, construct **and
fully consume the stream inside its async block**, requiring the terminal event;
returning an unconsumed stream or starting detached work defeats the intended
lifetime boundary. The guard's mutable borrow prevents review through that handle
while its invocation future exists.

```rust,no_run
use bourso_api::{replay_guard::ReplayGuard, OutcomeOperation};

async fn guarded<T, E>(
    guard: &mut ReplayGuard,
    call: impl FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>>>>,
) -> Result<Result<T, E>, bourso_api::replay_guard::GuardError> {
    guard.invoke(OutcomeOperation::Order, call).await
}
```

For an operation returning `Result<T, E>`, the output is
`Result<Result<T, E>, GuardError>`: the outer error is local guard failure, and
the inner result/error is unchanged. In particular, `OutcomeUnknown` downcasts,
context and privileged sensitive-reference access remain available. This helper
never logs or classifies the inner result. Consumers must protect their own
output/log sinks; do not log raw provider states, references or errors.

The journal is intentionally **possibly-not-emitted**: validation, authentication
or an earlier step can fail after reservation but before a bank mutation. Even a
nominal `SubmittedUnverified` response leaves it unresolved. No result is promoted
to bank acceptance/execution, and no retry is performed.

## Inspection and explicit review

`ReplayGuard::open(path)` acquires the domain lock and allows inspection of an
existing journal. `unresolved()` returns `Option<Attempt>` or a fixed local error.
A corrupt journal cannot be cleared through the review API; investigate manually,
never delete it merely to bypass blocked submission.

Only after independent bank review, invoke
`review(attempt.id(), ReviewDecision::NotApplied)` or
`review(attempt.id(), ReviewDecision::OperatorReviewed)` on the same locked handle.
The identifier must match the exact current journal. These decisions are operator
assertions, not bank states. Review makes no bank request and submits no subsequent
operation. A later invocation is a separate explicit action. An unsuccessful or
ambiguous release must not trigger automatic retry. No receipt or retained review
audit is stored; the consumer owns sensitive evidence and retention policy.

## Evidence and limits

Tests exercise public guarded order/transfer APIs against loopback fixtures,
reservation before first request, response loss, owner drop after complete final
POST, child-process termination before/after emission, reopen, concurrency,
private permissions, malformed storage and explicit review. Private fault tests
exercise partial-write/flush failure boundaries. These are local/mock tests, not
provider contract evidence. The no-retry observation window is bounded (150 ms).

Writes are flushed, **not fsynced**. There is no filesystem/controller/power-loss
durability guarantee, distributed coordination, network-filesystem lock guarantee,
provider idempotency or exactly-once execution. Bank order cancellation is a
separate mutation and is not covered by this helper or `OutcomeUnknown`'s current
scope. Provider positive/refusal/receipt semantics remain obligations of
`bourso-y3u.4/.6/.7/.8`; the local opt-in slice `.42` does not close them.
