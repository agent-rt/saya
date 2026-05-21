#!/usr/bin/env bash
# devloop — drive Saya via the DevServer (CDP-like JSON-RPC).
#
# Usage:
#   scripts/devloop.sh                     # default: query "chr", show snapshot
#   scripts/devloop.sh "1+2*3"             # custom query
#   scripts/devloop.sh "chr" submit        # type chr, hit Enter
#   scripts/devloop.sh "" clipboard        # snapshot clipboard panel
#
# Deterministic — no synthetic keystrokes, no focus races. Mutates and reads
# the AppState directly via the local TCP RPC server inside Saya.

set -euo pipefail
cd "$(dirname "$0")/.."

QUERY="${1:-chr}"
ACTION="${2:-snapshot}"   # snapshot | submit | clipboard

SAYA_BIN="target/debug/saya"
APP="output/Saya.app"

echo "==> build"
just build >/dev/null
cargo build -p saya-cli >/dev/null

echo "==> ensure Saya running"
if ! pgrep -f "Saya.app/Contents/MacOS/Saya" > /dev/null; then
    ABS_APP="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
    open "$ABS_APP"
    sleep 1.5
fi

echo "==> wait for dev server"
for _ in $(seq 1 50); do
    if "$SAYA_BIN" dev ping 2>/dev/null | grep -q pong; then
        break
    fi
    sleep 0.1
done

case "$ACTION" in
    clipboard)
        "$SAYA_BIN" dev panel.open --params '{"kind":"clipboard"}'
        echo "--- recent (head) ---"
        "$SAYA_BIN" dev clipboard.snapshot | jq '.result.recent[:5]'
        ;;
    snapshot|submit)
        "$SAYA_BIN" dev panel.open --params '{"kind":"launcher"}'
        "$SAYA_BIN" dev input.set --params "$(jq -nc --arg q "$QUERY" '{query:$q}')"
        echo "--- snapshot ---"
        "$SAYA_BIN" dev launcher.snapshot | jq '.result | {query, selected, items: .items[:5]}'
        if [ "$ACTION" = "submit" ]; then
            echo "--- submitting ---"
            "$SAYA_BIN" dev input.submit
        else
            "$SAYA_BIN" dev panel.close --params '{"kind":"launcher"}' >/dev/null
        fi
        ;;
    *)
        echo "unknown action: $ACTION"; exit 1
        ;;
esac
