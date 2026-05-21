#!/usr/bin/env bash
# Shared helpers for end-to-end tests against the Saya DevServer.
#
# Source from any scripts/test/*.sh:
#     source "$(dirname "$0")/lib.sh"

SAYA="${SAYA:-target/debug/saya}"
APP="${APP:-output/Saya.app}"

# ANSI colors (no-op if not a tty).
if [ -t 1 ]; then
    GREEN=$'\033[32m'; RED=$'\033[31m'; YELLOW=$'\033[33m'
    DIM=$'\033[2m'; BOLD=$'\033[1m'; RESET=$'\033[0m'
else
    GREEN=''; RED=''; YELLOW=''; DIM=''; BOLD=''; RESET=''
fi

# Counters set by run_tests
PASS=0
FAIL=0
FAILED_NAMES=()

# ---- RPC -----------------------------------------------------------------

# saya_rpc <method> [params_json]   → prints response JSON
saya_rpc() {
    local method="$1"
    local params="${2:-}"
    if [ -n "$params" ]; then
        "$SAYA" dev "$method" --params "$params"
    else
        "$SAYA" dev "$method"
    fi
}

# ---- Lifecycle -----------------------------------------------------------

ensure_saya_running() {
    if pgrep -f "Saya.app/Contents/MacOS/Saya" >/dev/null; then
        return 0
    fi
    local abs="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
    open "$abs"
    # Wait for the dev server.
    for _ in $(seq 1 50); do
        if "$SAYA" dev ping 2>/dev/null | grep -q pong; then
            return 0
        fi
        sleep 0.1
    done
    echo "${RED}ERR${RESET} dev server never came up" >&2
    exit 1
}

# ---- Assertions ----------------------------------------------------------

# expect <description> <expected> <actual>
expect() {
    local what="$1"; local want="$2"; local got="$3"
    if [ "$got" = "$want" ]; then
        echo "    ${GREEN}✓${RESET} ${DIM}${what}${RESET}: $got"
        return 0
    fi
    echo "    ${RED}✗${RESET} ${what}: expected ${BOLD}$want${RESET}, got ${BOLD}$got${RESET}"
    return 1
}

# expect_ge <description> <floor> <actual>
expect_ge() {
    local what="$1"; local floor="$2"; local got="$3"
    if [ "$got" -ge "$floor" ] 2>/dev/null; then
        echo "    ${GREEN}✓${RESET} ${DIM}${what}${RESET}: $got ≥ $floor"
        return 0
    fi
    echo "    ${RED}✗${RESET} ${what}: expected ≥ $floor, got ${BOLD}$got${RESET}"
    return 1
}

# ---- Runner --------------------------------------------------------------

run_tests() {
    for fn in $(declare -F | awk '{print $3}' | grep '^test_' | sort); do
        printf "${BOLD}▶ %s${RESET}\n" "$fn"
        if $fn; then
            PASS=$((PASS + 1))
        else
            FAIL=$((FAIL + 1))
            FAILED_NAMES+=("$fn")
        fi
        echo
    done
    printf "${BOLD}%d passed, %d failed${RESET}\n" "$PASS" "$FAIL"
    if [ "$FAIL" -gt 0 ]; then
        printf "${RED}Failed:${RESET}\n"
        for n in "${FAILED_NAMES[@]}"; do
            printf "  - %s\n" "$n"
        done
        return 1
    fi
}
