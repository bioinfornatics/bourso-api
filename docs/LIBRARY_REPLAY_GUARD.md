# Mandatory financial session — local, conservative

Public financial submissions require `bourso_api::FinancialSession`. There is no
public legacy/raw `BoursoWebClient::order`, `order_detailed`, `transfer_funds`,
`transfer_money` or `cancel_order`, including clients obtained through `get_client`,
`new` or `Default`. Cancellation is unavailable pending a separate safe contract.
Read/authentication APIs remain public. This is not provider idempotency, receipt
validation, automatic reconciliation or verified execution.

## Storage and coordination domain

Supply an **absolute path** to a trusted local Unix directory. Parent directories
must already exist; symlink ancestors and `..` components are refused. The helper
creates/tightens the directory to 0700 and regular files to 0600, rejects symlinks
and multiply-linked files, and does not reveal paths in its diagnostics. This is
not protection against malicious same-user filesystem replacement. Non-Unix
storage is refused; Windows ACL safety is not established.

All cooperating financial calls must use the same directory. A reserved order
blocks a transfer in that domain and vice versa. Changing the directory, deleting
files or using another machine bypasses the local domain. CLI and library use
this SAME `.42` guard and these files when configured with the same directory.
The CLI selects `BOURSO_DATA_DIR` or `~/.bourso`; library callers must pass that
absolute path explicitly. DCA retains a separate process journal/review.
The CLI refuses an unexpected `financial-unresolved.json` with a fixed local
error; no legacy journal migration or deletion is performed.

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

`FinancialSession::open(path)` owns a private `ReplayGuard`, with no extraction,
client accessor, `Deref` or bypass. Methods borrow the session mutably and take an
authenticated `&BoursoWebClient`. On the first poll, the session reserves by
exclusive creation, complete write and flush **before the entire financial
chain**, including its first GET or intermediate POST. An unpolled future does
not reserve. Every result, panic, cancellation/drop or crash retains the journal.
Reservation errors prevent the chain; partial/invalid journals stay blocking.

```rust,no_run
use bourso_api::{FinancialSession, account::Account, client::{BoursoWebClient,
    trade::order::{OrderData, OrderSide, OrderSubmission},
    transfer::{money::Money, TransferProgress}}, replay_guard::GuardError};

async fn submit_order(session: &mut FinancialSession, client: &BoursoWebClient,
    account: &Account) -> Result<anyhow::Result<OrderSubmission>, GuardError> {
    session.order(client, OrderSide::Buy, account, "TEST", 2, None::<OrderData>).await
}
async fn transfer(session: &mut FinancialSession, client: &BoursoWebClient,
    amount: Money, from: Account, to: Account)
    -> Result<anyhow::Result<TransferProgress>, GuardError> {
    session.transfer_money(client, amount, from, to, None).await
}
```

`order(&mut self, client, side, account, symbol, quantity, order_data)` returns
`Result<anyhow::Result<OrderSubmission>, GuardError>`.
`transfer_money(&mut self, client, amount, from, to, reason)` returns
`Result<anyhow::Result<TransferProgress>, GuardError>`. It constructs and fully
consumes the internal stream and requires the final `SubmittedUnverified` event;
no lazy stream escapes. The future retains the mutable borrow, excluding review
while alive. No float compatibility entry point is provided.

Only outer errors are local pre-invocation failures. Inner errors (including
`OutcomeUnknown` downcasts and sensitive-reference access) remain unchanged.
Neither layer logs bank results; callers must protect output/log sinks.
`ReplayGuard` remains a public generic utility, but cannot expose or invoke any
raw financial methods. Do not wrap the session in a second reservation in the
same domain: the session already reserves it.

The journal is intentionally **possibly-not-emitted**: validation
or an earlier financial step can fail after reservation but before a bank mutation. Even a
nominal `SubmittedUnverified` response leaves it unresolved. No result is promoted
to bank acceptance/execution, and no retry is performed.

## Inspection and explicit review

`FinancialSession::open(path)` acquires the domain lock and allows inspection of an
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
provider idempotency or exactly-once execution. Public bank order cancellation is unavailable; its provider contract remains unverified. Provider positive/refusal/receipt semantics remain obligations of
`bourso-y3u.4/.6/.7/.8`; the local session slice `.48` does not close them.
