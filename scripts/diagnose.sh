#!/usr/bin/env bash
# Capture a complete log slice while the user reproduces an issue.
# Usage:
#   scripts/diagnose.sh
# Then in another window:
#   - Reproduce the bug (open panel, type, etc.)
#   - Press Enter in this script to stop capture
# Outputs to /tmp/saya-diagnose-<timestamp>.log for sharing.

set -euo pipefail
cd "$(dirname "$0")/.."

LOG="$HOME/Library/Logs/Saya/saya.log"
OUT="/tmp/saya-diagnose-$(date +%Y%m%d-%H%M%S).log"

mkdir -p "$(dirname "$LOG")"

echo "==> 1. quit any running Saya (so we restart cleanly with full debug logging)"
pkill -f "Saya.app/Contents/MacOS/Saya" 2>/dev/null || true
sleep 0.5

echo "==> 2. mark start of capture in log"
echo "===DIAGNOSE START $(date)===" >> "$LOG"

echo "==> 3. relaunching Saya with DEBUG-level FFI logging"
echo "    (This launches whichever Saya.app the system has registered for the bundle ID.)"
# Force debug log via env var inherited through `open`.
SAYA_LOG="saya_ffi=debug,saya_core=debug,saya_ui=debug" \
  open -a Saya || open -a /Applications/Saya.app

cat <<EOF

==> 4. Reproduce the bug now.
    For example: press ⌥ Space → type "chr" → wait → press Esc.

==> Press ENTER when you're done capturing.
EOF
read -r _

echo "===DIAGNOSE END $(date)===" >> "$LOG"

# Slice the log from the START marker to here.
awk '/===DIAGNOSE START/{found=1} found' "$LOG" > "$OUT"

echo
echo "==> Diagnostic log written to: $OUT"
echo "    Size:  $(wc -l < "$OUT") lines"
echo
echo "Send this file (or paste relevant excerpt) to the maintainer."
