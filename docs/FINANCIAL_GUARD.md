# Financial CLI replay barrier (local, conservative)

The CLI's `trade order new` and `transfer` commands now use a shared local replay
barrier. Their existing API submissions remain enabled. This is **not** bank
idempotency, receipt validation, automated reconciliation or verified execution.
No provider success/rejection states are invented.

## Scope and storage

The guard uses `~/.bourso` or the absolute directory selected by `BOURSO_DATA_DIR`.
Use the **same trusted directory** for all financial CLI invocations. Changing the
directory, deleting files, using another machine or calling `bourso_api` directly
bypasses this protection. Different credentials, accounts, symbols and operations
share one barrier in that directory: an unresolved order also blocks transfers.
The launcher documented in [DCA.md](DCA.md) has a **separate** journal; if both
barriers are unresolved, review each independently. Neither clears the other.

- `financial.lock`: private stable regular file, held with an exclusive OS lock
  across authentication, financial invocation and result handling. Never remove
  this file, even after a crash: the OS releases the lock when the process dies.
- `financial-unresolved.json`: contains only schema version, operation kind and a
  locally generated attempt ID (not a bank ID or idempotency key). It is created
  exclusively and written **before** invoking `order_detailed` or constructing
  and polling `transfer_money`. An existing, malformed, partial, unreadable or
  unsafe journal blocks invocation. No error handler or destructor clears it.

Existing private-storage helpers create/tighten Unix directories to `0700` and
regular files to `0600`; symlinks and Unix multiply-linked files are refused.
The directory and its ancestors must be trusted. This is not protection against
malicious same-user path races. Windows ACLs are neither established nor tested.

The guard has no final-POST boundary hook. An invocation error or cancellation
may occur **before** any bank mutation: the unresolved attempt is explicitly
**possibly-not-emitted**. This conservative false positive is intentional. A
settings/argument/authentication/account-selection error before the financial API
is invoked does not create an unresolved journal. A failed journal write also
never invokes the API; any partial file is retained and blocks later attempts.

## Results and exit status

Financial result lines go directly to **stderr**, independent of `RUST_LOG`.
Financial commands do not initialize tracing (including the persistent log), to
avoid exposing raw bank IDs, responses, URLs or errors through diagnostics.
Authentication may still prompt for a password, show a QR challenge or require
approval in the bank application; these are interactive authentication, not
financial result diagnostics. Review output goes to stdout.

| Exit | Result | Meaning |
| --- | --- | --- |
| `2` | `financial-result: not-emitted` | This command did not invoke the financial API (invalid input, preflight/storage failure, or prior unresolved attempt). It makes **no assertion about an earlier attempt**. |
| `3` | `financial-result: response-unverified` | Financial API finished with a submission response. Acceptance/execution is **not established**. Journal remains unresolved. |
| `4` | `financial-result: outcome-unknown` | Financial API returned an error or transfer ended without its terminal response. The operation may have applied, or may not have been emitted. Journal remains unresolved. |
| `0` | review/help only | Local administrative operation; never financial success. |

On termination/crash there may be no result line. After the pre-invocation journal
write, replay remains blocked. Ordinary API errors (including `OutcomeUnknown`)
and apparently successful responses never automatically release the barrier.
Do not automate retries from these exit codes. Bank APIs and the legacy order
API remain available without this CLI guard; direct library consumers must
implement their own safe orchestration and redaction. `parse_matches` returns a
typed local `financial_guard::Report` error for completed guarded attempts, even
for response-unverified; it is not a bank success API.

## Explicit operator review (no bank request)

First stop submissions and reconcile independently in the bank, including pending
orders/transfers. If the outcome remains unclear, **leave the journal unresolved**.
Inspect the local attempt without loading credentials or calling the bank:

```sh
bourso-cli financial-review
```

Only after independent review, release the matching local barrier:

```sh
# Independent evidence establishes the attempt was not applied:
bourso-cli financial-review --attempt LOCAL_ATTEMPT_ID --decision not-applied

# An operator has reconciled the attempt and accepts responsibility for release:
bourso-cli financial-review --attempt LOCAL_ATTEMPT_ID --decision operator-reviewed
```

Both commands require the exact current attempt ID and hold the same exclusive
lock; review cannot race an active financial command. The decision is an operator
assertion, **not** a bank state, and invokes no financial operation. Successful
review deletes only the matching unresolved journal. There is no retained audit
history or authenticated receipt storage; keep review evidence securely elsewhere.
A subsequent operation must be invoked separately. Wrong IDs and malformed
journals are not cleared. Corrupt/partial journals require manual investigation;
do not delete them merely to bypass a blocked operation.

## Durability and verification limits

Writes are flushed before invocation but **not fsynced**. No filesystem/power-loss
durability guarantee is made; disk/controller loss, external mutation and network
filesystem locking semantics are outside the tested scope. Local locking and a
single journal do not provide global exactly-once execution. There is no
whole-command timeout; a hung process retains its lock until terminated.

Local tests use fake futures, cancellation, process termination, concurrent lock
attempts, temporary storage failures and fake result presentation with logs off.
They exercise no bank requests. Unix permissions are tested on Linux, not Windows
ACLs or native macOS execution. Real provider confirmation contracts and trusted
reconciliation remain unresolved; this guard does not close those acceptance gates.

```sh
export PATH="$HOME/.rustup/toolchains/1.96.1-x86_64-unknown-linux-gnu/bin:$PATH"
cargo test --locked -p bourso-cli financial_guard
cargo test --locked -p bourso-cli --test financial_guard_cli
```
