# DCA launcher: partial local safety, not unattended trading

`bourso-launchd.sh` is a **semi-interactive attempt guard**, despite its historical
name. It invokes the configured executable in the foreground, inheriting stdin,
stdout and stderr. It does not open Terminal, install launchd/cron jobs, bypass MFA,
verify financial results, or guarantee exactly-once execution at the bank.

This is partial work for `bourso-y3u.27`, **not task closure or production
acceptance**. Provider contracts (`.4`), reliable financial confirmation (`.6`)
and unknown-outcome handling (`.8`) remain unresolved. An exit code, order ID,
HTTP response, or CLI message is not a trusted bank receipt. Do not use this as
unattended DCA automation. Local fake tests establish no live-bank compatibility.

## Configuration and execution boundary

The script retains the original symbol/account/quantity defaults and the
`/path/to/your/bourso-cli` placeholder. **Configure and review all four** before
any real use; the original account is not a suggested account for you. Environment
variables `SCRIPT_PATH`, `SYMBOL`, `ACCOUNT`, and `QUANTITY` override the defaults.
`SCRIPT_PATH` must be an absolute executable-file path. No additional financial
acknowledgment switch gates an otherwise configured first invocation: it really
can submit an order. There is no dry-run mode.

- Default mode, also selectable with `--mode interactive`, requires stdin to be
  a TTY. The actual child runs directly and can prompt or wait for device approval.
- `--mode existing-credentials` explicitly selects an already configured readable
  `CREDENTIALS_FILE`. It passes that path as the CLI's root `--credentials` option
  and can run without stdin TTY. It neither creates a credential file nor checks
  that its contents avoid prompting. **Existing credentials do not imply
  unattended operation**: password input or MFA/device interaction may still be
  required; there is no whole-command deadline here.
- No new secret storage is introduced. Do not put passwords into this script,
  command arguments, journal, receipt references, or source control. Protect
  any pre-existing credentials independently. Child output is not captured in
  this state journal; terminal output and the CLI's own logs remain separate.

`bash bourso-launchd.sh --help` only describes options; it does not initialize
state or invoke the CLI. Execution is normally `bash bourso-launchd.sh` **after
local configuration and independent risk review**; do not run this merely to
check configuration. A scheduler without TTY fails in default mode rather than
opening an asynchronous Terminal window. No scheduling installation is endorsed.

## Local state and repeat protection

The default directory is `$HOME/.bourso-cli/dca`; `DCA_STATE_DIR` can select an
absolute path under a trusted parent. Use the **same directory for all invocations
of a strategy**, even if you change account, symbol, or executable. A different
state directory, direct CLI invocation, or another machine bypasses this local
protection. This is not server idempotency or a distributed lock.

Directories created by the script use mode `700`; new files use `600` through
`umask 077` and `mktemp`. Existing state must be owned by the current user and have
those exact permissions; symlink state roots/files are rejected. Existing
permissions are never tightened or broadened automatically. Parent directories
must be trusted: this is not protection against hostile same-user changes or
symlink races in ancestor directories. Atomic file replacement is within the
same directory; no fsync/power-loss durability guarantee is made.

| Object / state | Meaning |
| --- | --- |
| `lock/` | Atomic `mkdir` lock held across state checks, child execution and result recording. Concurrent runs and review commands refuse it. |
| `attempt`, `state=unknown` | Written **before** invoking the child. `exit_status=not-observed` initially; an observed nonzero exit stays unknown, even on login failure or cancellation. |
| `attempt`, `state=submitted-unverified` | Child exited zero; submission is still unverified. It does **not** establish acceptance or execution. |
| `reviewed-confirmed` / `reviewed-not-applied` | Explicit operator reconciliation of the matching attempt, never inferred from a child exit. |
| `last_operator_confirmed` | Local epoch time of explicit operator confirmation against a trusted receipt. Only this starts the seven-day interval. |
| `reviewed.*` | Private snapshots of previous reviewed attempts, preserved before a new attempt. No automatic history pruning. |

Both unknown and submitted-unverified states block every repeat until review.
Malformed state also blocks; never delete it to get an execution through.
An interval skip exits `0` without executing. A recorded operator review exits
`0` without executing. An executed child (zero **or** nonzero), unresolved attempt,
active/stale lock, or future timestamp exits `3`. Usage/configuration/state errors
normally exit `2`; shell/filesystem failures may return other nonzero statuses.
**Wrapper exit zero never means financial success.** Child exit status is kept in
the private journal, not propagated as a financial result.

The old `$HOME/.bourso-cli/last_run` file is neither deleted nor trusted/imported:
the old launcher wrote it before knowing the outcome. Before switching from the
old launcher, stop old scheduling, inspect prior bank activity independently and
account for any prior order/interval yourself. A fresh state directory cannot
know old attempts; do not treat migration as permission for a new purchase.
Other local state and credential directories are left untouched.

## Explicit reconciliation (never an automatic retry)

1. Stop triggering further invocations. Read the current private `attempt` file
   and its `id`. Check the bank independently, including pending orders and any
   still-running child. A missing response or CLI error does not prove rejection.
2. If the bank outcome remains unclear, **leave the attempt unresolved**. Do not
   acknowledge it, change state directories, or replay the financial command.
3. Only after independent review, use the exact current attempt ID:

   ```sh
   # Records operator-confirmed receipt review, NOT a new trade.
   bash bourso-launchd.sh --acknowledge-reviewed-attempt ATTEMPT_ID \
     --outcome confirmed --receipt-reference LOCAL_NON_SECRET_REFERENCE
   ```

   `LOCAL_NON_SECRET_REFERENCE` is an opaque reference to evidence you keep
   securely elsewhere, not the receipt itself, an account ID, or a password.
   Letters, digits, dot, underscore and hyphen are accepted. The script cannot
   authenticate the reference or verify the bank; the operator must do that.
   It timestamps **the review**, not the bank's trade/fill time, and waits seven
   days from that review. Confirmation writes the interval first; interruption
   before the reviewed state is written remains conservatively blocked.

4. If independent evidence establishes that the bank **did not apply** the
   attempt, the explicit alternative is:

   ```sh
   bash bourso-launchd.sh --acknowledge-reviewed-attempt ATTEMPT_ID \
     --outcome not-applied
   ```

   This does not execute a trade and does not start/reset the weekly interval.
   It permits a later, separately invoked attempt if any previous interval has
   elapsed. Do not use it for an ambiguous or merely apparently failed request.
   Review options cannot be combined with execution mode options. A wrong ID,
   already reviewed attempt, or missing required receipt reference is refused.

### Interrupted process / stale lock

Ordinary exit and catchable signals release this invocation's empty lock; the
attempt journal remains. SIGKILL or machine failure may leave `lock/`. **There
is no PID-based stale detection or automatic lock deletion.** A foreground child
may outlive an interrupted supervisor. Before any manual recovery, establish
that no wrapper or child can still submit work and inspect the bank independently.
A PID's absence alone is insufficient financial evidence.

Only after that inspection may the operator manually remove the **empty**
`lock/` directory (e.g. `rmdir` on the specific inspected path). Do not recursively
remove the state directory or journal. Then reconcile using the exact attempt ID
as above; removing a stale lock alone never clears an unknown attempt. Private
`.write.*` files from interrupted writes may be retained for manual inspection;
they are not automatically promoted to authoritative state. Inconsistent state,
filesystem damage or uncertain recovery needs manual investigation, not replay.

## Local verification

Run from the repository root:

```sh
bash -n bourso-launchd.sh scripts/test-dca.sh
bash scripts/test-dca.sh
```

Tests require Bash and Python 3, use isolated temporary HOME/state directories and
fake executable fixtures only, and remove only their own fixture directory.
They cover first-run directory creation, permissions, direct child observation,
zero/nonzero unknown results, repeat blocking, explicit reconciliation, the
operator-confirmed interval, preserved reviewed history, concurrency, SIGKILL
stale-lock recovery, TTY/configuration rejection and malformed/unsafe state.
They create no real credentials and make no bank calls. Bash 3-compatible syntax
and BSD/GNU stat handling target macOS portability; a Linux run does **not** prove
macOS/launchd operation. Trusted automatic financial confirmation is missing,
so passing this suite cannot close `.27`, `.4`, `.6`, `.8`, or a production gate.
