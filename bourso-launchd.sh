#!/bin/bash
# Semi-interactive local attempt guard, NOT unattended DCA or financial confirmation.
# Configure these locally or through the environment; no credentials are stored here.
set -euo pipefail
umask 077
SCRIPT_PATH=${SCRIPT_PATH:-/path/to/your/bourso-cli}
SYMBOL=${SYMBOL:-1rTCW8}
ACCOUNT=${ACCOUNT:-a583f3c5842c34fb00b408486ef493e0}
QUANTITY=${QUANTITY:-1}
DCA_STATE_DIR=${DCA_STATE_DIR:-"$HOME/.bourso-cli/dca"}
INTERVAL=$((7 * 24 * 60 * 60))

usage() {
    cat <<'HELP'
Usage: bourso-launchd.sh [--mode interactive|existing-credentials]
       bourso-launchd.sh --acknowledge-reviewed-attempt ATTEMPT_ID \
         --outcome confirmed|not-applied [--receipt-reference LOCAL_REFERENCE]
Default mode: interactive (stdin must be a TTY). Existing-credentials mode uses
CREDENTIALS_FILE, an already configured file; it does not bypass MFA.
A CLI exit is never financial proof. Every attempt blocks another invocation
until the operator independently reconciles it. Review never executes an order.
Confirmed review requires a trusted bank receipt, represented by a non-secret
local reference. Only that explicit review starts the seven-day interval.
HELP
}
fail() { printf '%s\n' "$*" >&2; exit 2; }
blocked() { printf '%s\n' "$*" >&2; exit 3; }
mode=interactive
review_id=
outcome=
receipt=
mode_given=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --help|-h) usage; exit 0 ;;
        --mode) [ "$#" -ge 2 ] || fail 'Missing mode'; mode=$2; mode_given=true; shift 2 ;;
        --acknowledge-reviewed-attempt) [ "$#" -ge 2 ] && [ -n "$2" ] || fail 'Missing attempt ID'; review_id=$2; shift 2 ;;
        --outcome) [ "$#" -ge 2 ] || fail 'Missing outcome'; outcome=$2; shift 2 ;;
        --receipt-reference) [ "$#" -ge 2 ] || fail 'Missing receipt reference'; receipt=$2; shift 2 ;;
        *) fail 'Unknown argument; use --help' ;;
    esac
done
case "$mode" in interactive|existing-credentials) ;; *) fail 'Invalid mode' ;; esac
if [ -n "$review_id" ]; then
    [ "$mode_given" = false ] || fail 'Review and execution modes cannot be combined'
    case "$outcome" in confirmed|not-applied) ;; *) fail 'Review requires --outcome confirmed|not-applied' ;; esac
    case "$review_id" in *[!a-zA-Z0-9._-]*) fail 'Invalid attempt ID' ;; esac
    if [ "$outcome" = confirmed ]; then
        case "$receipt" in ''|*[!a-zA-Z0-9._-]*) fail 'Use a non-secret local receipt reference (letters, digits, dot, underscore, hyphen)' ;; esac
    else
        [ -z "$receipt" ] || fail 'A not-applied review must not include a receipt'
    fi
else
    [ -z "$outcome$receipt" ] || fail 'Review options require --acknowledge-reviewed-attempt'
fi

# BSD/macOS and GNU stat. Do not repair permissions or follow state symlinks.
permissions() {
    stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"
}
private_dir() {
    [ ! -L "$1" ] && [ -d "$1" ] && [ -O "$1" ] && [ "$(permissions "$1")" = 700 ]
}
private_file() {
    [ ! -L "$1" ] && [ -f "$1" ] && [ -O "$1" ] && [ "$(permissions "$1")" = 600 ]
}
case "$DCA_STATE_DIR" in /*) ;; *) fail 'DCA_STATE_DIR must be absolute' ;; esac
[ ! -L "$DCA_STATE_DIR" ] || fail 'State directory must not be a symlink'
if [ ! -e "$DCA_STATE_DIR" ]; then mkdir -p "$DCA_STATE_DIR"; fi
private_dir "$DCA_STATE_DIR" || fail 'State must be an owned private directory (700); existing permissions are not changed'
lock="$DCA_STATE_DIR/lock"
if ! mkdir "$lock" 2>/dev/null; then
    blocked 'Lock present: active or interrupted run. Never auto-remove it; inspect processes and reconcile with the bank first.'
fi
# An interrupted run keeps its lock, even if its child later exits. There is no
# automatic stale recovery: cancellation cannot establish a bank outcome.
keep_lock=false
child_pid=
cleanup() {
    if [ "$keep_lock" = false ]; then rmdir "$lock" 2>/dev/null || :; fi
}
interrupt() {
    # Ignore repeated cancellation while forwarding; never escalate or retry.
    trap '' INT TERM HUP
    keep_lock=true
    if [ -n "$child_pid" ]; then
        # Normalize cancellation to TERM: asynchronous children can inherit
        # SIGINT ignored from Bash. A child may ignore TERM too; retain the lock.
        kill -TERM "$child_pid" 2>/dev/null || :
    fi
    exit "$1"
}
trap cleanup EXIT
trap 'interrupt 130' INT
trap 'interrupt 143' TERM
trap 'interrupt 129' HUP
attempt="$DCA_STATE_DIR/attempt"
confirmed="$DCA_STATE_DIR/last_operator_confirmed"
for file in "$attempt" "$confirmed"; do
    if [ -e "$file" ] || [ -L "$file" ]; then
        private_file "$file" || fail 'Unsafe state file; preserve it and inspect manually'
    fi
done
# Atomic replacement in the same private directory; interrupted temp files stay
# private and may be inspected. This is not an fsync/power-loss durability claim.
atomic_write() {
    local destination=$1 temporary
    temporary=$(mktemp "$DCA_STATE_DIR/.write.XXXXXXXX")
    cat > "$temporary"
    mv -f "$temporary" "$destination"
}
field() { sed -n "s/^$1=//p" "$attempt"; }
now=$(date +%s)
state=
id=
started=
if [ -f "$attempt" ]; then
    state=$(field state)
    id=$(field id)
    started=$(field started_at)
    case "$id" in ''|*[!a-zA-Z0-9._-]*) fail 'Malformed attempt ID; manual inspection required' ;; esac
    case "$started" in ''|*[!0-9]*) fail 'Malformed attempt time; manual inspection required' ;; esac
    case "$state" in unknown|submitted-unverified|reviewed-confirmed|reviewed-not-applied) ;; *) fail 'Malformed attempt state; manual inspection required' ;; esac
fi
if [ -n "$review_id" ]; then
    [ -n "$id" ] && [ "$id" = "$review_id" ] || fail 'Review ID does not match current attempt'
    case "$state" in unknown|submitted-unverified) ;; *) fail 'Attempt already reviewed' ;; esac
    if [ "$outcome" = confirmed ]; then
        # Timestamp first: interruption before the reviewed record still blocks.
        printf '%s\n' "$now" | atomic_write "$confirmed"
    fi
    {
        cat "$attempt" | sed '/^state=/d'
        printf 'state=reviewed-%s\nreviewed_at=%s\n' "$outcome" "$now"
        [ -z "$receipt" ] || printf 'receipt_reference=%s\n' "$receipt"
        :
    } | atomic_write "$attempt"
    printf 'Operator review recorded; no order executed. Financial verification is the operator responsibility.\n'
    exit 0
fi
case "$state" in
    unknown|submitted-unverified) blocked "Attempt $id remains $state; independently reconcile, then use --acknowledge-reviewed-attempt." ;;
esac
if [ -f "$confirmed" ]; then
    last=$(cat "$confirmed")
    case "$last" in ''|*[!0-9]*) fail 'Invalid operator-confirmed timestamp' ;; esac
    [ "${#last}" -le 10 ] || fail 'Invalid operator-confirmed timestamp'
    last=$((10#$last))
    if [ "$last" -gt "$now" ]; then blocked 'Operator timestamp is in the future; inspect clock/state, do not retry'; fi
    if [ $((now - last)) -lt "$INTERVAL" ]; then
        printf 'Skipped: seven days have not elapsed since operator confirmation.\n'
        exit 0
    fi
fi
[ "$SCRIPT_PATH" != /path/to/your/bourso-cli ] || fail 'Configure SCRIPT_PATH with the intended CLI executable'
case "$SCRIPT_PATH" in /*) ;; *) fail 'SCRIPT_PATH must be absolute' ;; esac
[ -f "$SCRIPT_PATH" ] && [ -x "$SCRIPT_PATH" ] || fail 'SCRIPT_PATH is not an executable file'
[ -n "$SYMBOL" ] && [ -n "$ACCOUNT" ] || fail 'Configure symbol and account'
case "$QUANTITY" in ''|*[!0-9]*|0) fail 'Quantity must be a positive integer' ;; esac
args=(trade order new --side buy --symbol "$SYMBOL" --account "$ACCOUNT" --quantity "$QUANTITY")
if [ "$mode" = interactive ]; then
    [ -t 0 ] || fail 'Interactive mode requires stdin TTY; no asynchronous Terminal launch is performed'
else
    [ -n "${CREDENTIALS_FILE:-}" ] && [ -f "$CREDENTIALS_FILE" ] && [ -r "$CREDENTIALS_FILE" ] || fail 'Existing-credentials mode requires readable CREDENTIALS_FILE; no file is created'
    args=(--credentials "$CREDENTIALS_FILE" "${args[@]}")
fi
# Preserve previous reviewed records, without deleting or truncating user data.
if [ -f "$attempt" ]; then
    archive=$(mktemp "$DCA_STATE_DIR/reviewed.XXXXXXXX")
    cat "$attempt" > "$archive"
fi
id="$(date +%s).$$"
started=$now
printf 'id=%s\nstate=unknown\nstarted_at=%s\nexit_status=not-observed\n' "$id" "$started" | atomic_write "$attempt"
printf 'Starting attempt %s (%s); bank outcome will remain unverified.\n' "$id" "$mode" >&2
# Explicit stdin preserves interactive prompts (Bash otherwise redirects an
# asynchronous command's stdin). Waiting on the actual child keeps this wrapper
# synchronous, but unlike a foreground external command, wait responds to traps.
# Keep the lock from BEFORE launch through wait, including the PID-assignment
# window: if interrupted there, manual recovery must inspect the live child.
keep_lock=true
"$SCRIPT_PATH" "${args[@]}" <&0 &
child_pid=$!
status=0
wait "$child_pid" || status=$?
child_pid=
keep_lock=false
state=unknown
[ "$status" -ne 0 ] || state=submitted-unverified
printf 'id=%s\nstate=%s\nstarted_at=%s\nexit_status=%s\nobserved_at=%s\n' "$id" "$state" "$started" "$status" "$(date +%s)" | atomic_write "$attempt"
printf 'Attempt %s: %s, CLI exit %s. Do not retry until independently reviewed.\n' "$id" "$state" "$status" >&2
# This status signals reconciliation needed, not child success or bank failure.
exit 3
