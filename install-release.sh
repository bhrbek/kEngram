#!/usr/bin/env bash
# install-release.sh — atomic kengram binary install.
# Usage: install-release.sh <candidate-binary> <expected-sha256> [--restart]
#
# Why this exists (lane kengram-release-install-truncates-live-binary-before-restart):
# the previous release path was a bare `cp` onto the live path — open(O_TRUNC) on the
# running binary's inode. Jones directly observed the live path at 0 bytes / e3b0 (empty
# sha) mid-install while the candidate sat complete beside it. This script guarantees the
# live path is only ever OLD bytes or NEW bytes:
#   copy candidate -> same-directory temp (same filesystem BY CONSTRUCTION, so the final
#   rename is atomic rename(2)) -> fsync temp -> hash-verify the TEMP COPY (not the
#   source; the copy is what gets installed) -> rename over live -> fsync directory ->
#   hash readback of the LIVE path -> only then, optionally, restart.
# The live path is never opened for write. On ANY failure the temp is removed and the
# old binary keeps serving.
set -euo pipefail

CANDIDATE="${1:?usage: install-release.sh <candidate-binary> <expected-sha256> [--restart]}"
EXPECTED_SHA="${2:?expected sha256 required — a release install without a pinned hash is not a release}"
DO_RESTART="${3:-}"

LIVE="${KENGRAM_LIVE_BINARY:-$HOME/argus/kengram/target/release/kengram}"
SERVICE_LABEL="${KENGRAM_SERVICE_LABEL:-com.yetiwks.argus-kengram}"

fail() { echo "install-release: $*" >&2; exit 1; }

[ -f "$CANDIDATE" ] || fail "candidate not a file: $CANDIDATE"
[ -s "$CANDIDATE" ] || fail "candidate is EMPTY: $CANDIDATE"
case "$EXPECTED_SHA" in
  *[!0-9a-f]*|"") fail "expected sha256 must be 64 lowercase hex chars" ;;
esac
[ "${#EXPECTED_SHA}" -eq 64 ] || fail "expected sha256 must be 64 hex chars (got ${#EXPECTED_SHA})"

LIVE_DIR="$(dirname "$LIVE")"
[ -d "$LIVE_DIR" ] || fail "live dir missing: $LIVE_DIR"

TMP="$LIVE_DIR/.kengram-install.$$.tmp"
cleanup() { rm -f "$TMP"; }
trap cleanup EXIT

# 1. Copy candidate to same-directory temp (never the live path).
cp "$CANDIDATE" "$TMP"
chmod 755 "$TMP"

# 2. fsync the temp copy — the bytes must be durable BEFORE they can become the live name.
/usr/bin/python3 - "$TMP" <<'PY'
import os, sys
fd = os.open(sys.argv[1], os.O_RDONLY)
os.fsync(fd)
os.close(fd)
PY

# 3. Verify the COPY (not the source): this is the artifact that will serve.
TMP_SHA="$(shasum -a 256 "$TMP" | cut -d' ' -f1)"
[ "$TMP_SHA" = "$EXPECTED_SHA" ] || fail "temp-copy sha mismatch: got $TMP_SHA want $EXPECTED_SHA — live binary UNTOUCHED"

OLD_SHA="none"
[ -f "$LIVE" ] && OLD_SHA="$(shasum -a 256 "$LIVE" | cut -d' ' -f1)"

# 4. Atomic cutover: rename(2), same filesystem by construction. The live path goes
#    OLD -> NEW with no intermediate state observable at any instant.
mv -f "$TMP" "$LIVE"
trap - EXIT

# 5. fsync the directory so the rename itself is durable.
/usr/bin/python3 - "$LIVE_DIR" <<'PY'
import os, sys
fd = os.open(sys.argv[1], os.O_RDONLY)
os.fsync(fd)
os.close(fd)
PY

# 6. Readback from the LIVE path — the claim is about what is installed, not what was sent.
LIVE_SHA="$(shasum -a 256 "$LIVE" | cut -d' ' -f1)"
[ "$LIVE_SHA" = "$EXPECTED_SHA" ] || fail "POST-RENAME readback mismatch: $LIVE_SHA — investigate before restart"
LIVE_SIZE="$(stat -f%z "$LIVE")"
LIVE_MODE="$(stat -f%Lp "$LIVE")"

echo "INSTALLED live=$LIVE sha256=$LIVE_SHA size=$LIVE_SIZE mode=$LIVE_MODE old_sha256=$OLD_SHA"

# 7. Restart is a SEPARATE, explicit act — and it is the LAST step, never earlier.
if [ "$DO_RESTART" = "--restart" ]; then
  launchctl kickstart -k "gui/$(id -u)/$SERVICE_LABEL"
  echo "RESTARTED $SERVICE_LABEL"
else
  echo "NOT restarted. When released: launchctl kickstart -k gui/$(id -u)/$SERVICE_LABEL"
fi
