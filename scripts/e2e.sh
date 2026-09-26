#!/usr/bin/env bash
# End-to-end smoke (plan T7.1/T7.2). Requires: local herdr running + pi.
# Usage: ./scripts/e2e.sh
set -euo pipefail

BIN=${TOWER_BIN:-./target/debug/tower}
HOME_DIR=$(mktemp -d /tmp/tower-e2e.XXXXXX)
PORT=${TOWER_PORT:-8266}
RUN_ID=$(date +%s)-$$
AGENT="tower-e2e-$RUN_ID"
export TOWER_HOME="$HOME_DIR"

cleanup() {
  [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null || true
  # remove smoke agents from herdr if any remain
  for pane in $(herdr agent list 2>/dev/null | python3 -c \
    'import json,sys
print(" ".join(a["pane_id"] for a in json.load(sys.stdin)["result"]["agents"] if a["name"].startswith("tower-e2e")))' \
    2>/dev/null || true); do
    herdr pane close "$pane" >/dev/null 2>&1 || true
  done
  rm -rf "$HOME_DIR"
}
trap cleanup EXIT

step() { echo; echo "== $* =="; }

step "build"
cargo build -p tower-server >/dev/null

step "start server (fresh home)"
setsid "$BIN" serve >"$HOME_DIR/server.log" 2>&1 &
SERVER_PID=$!
sleep 2
"$BIN" doctor

step "spawn pi agent"
"$BIN" spawn "$AGENT" --kind pi
sleep 1
"$BIN" ps

step "prompt + read-back (pi provider auth may be unset — output on screen is the assertion)"
"$BIN" prompt "$AGENT" 'reply with the word acknowledged'
sleep 3
OUT=$("$BIN" read "$AGENT")
echo "$OUT" | head -6
# prompt delivery is observable: the text appears in the pane (or pi's
# no-auth error screen — both prove driver → harness delivery)
if ! grep -qiE 'acknowledged|api key' <<<"$OUT"; then
  echo "FAIL: expected prompt echo or pi auth error in output"
  exit 1
fi
echo "prompt delivered and read back"

step "state visible in ps"
"$BIN" ps

step "restart server mid-run (T7.2)"
kill "$SERVER_PID"; wait "$SERVER_PID" 2>/dev/null || true
sleep 1
setsid "$BIN" serve >>"$HOME_DIR/server.log" 2>&1 &
SERVER_PID=$!
sleep 2
"$BIN" ps
# agent row must survive restart and rebind to the same pane
"$BIN" ps | grep -q "$AGENT" || { echo "FAIL: agent row lost after restart"; exit 1; }

step "event log unbroken across restarts"
TOKEN=$(cat "$HOME_DIR/token")
STARTS=$(curl -sN --max-time 5 -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/v1/events?cursor=0&filter=type:server.started" \
  | grep -c server.started || true)
echo "server.started events: $STARTS"
[[ "$STARTS" -ge 2 ]] || { echo "FAIL: expected 2 server.started events"; exit 1; }

step "stop agent, verify row survives (seat)"
"$BIN" stop "$AGENT"
"$BIN" ps | grep -q "$AGENT" || { echo "FAIL: seat row lost"; exit 1; }
"$BIN" ps

step "E2E SMOKE PASSED"