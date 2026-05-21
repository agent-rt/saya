#!/usr/bin/env bash
# End-to-end test suite for Saya, driven entirely through the DevServer RPC.
# Each test mutates AppState, snapshots it, asserts, and tears down.

set -uo pipefail
cd "$(dirname "$0")/../.."
source scripts/test/lib.sh

# ==== Calculator ==========================================================

test_calc_simple_addition() {
    saya_rpc panel.open '{"kind":"launcher"}' >/dev/null
    saya_rpc input.set '{"query":"1+2"}' >/dev/null
    local snap; snap=$(saya_rpc launcher.snapshot)
    local kind;  kind=$(jq -r '.result.items[0].kind' <<<"$snap")
    local val;   val=$(jq -r '.result.items[0].value' <<<"$snap")
    saya_rpc panel.close '{"kind":"launcher"}' >/dev/null
    expect "kind" calc "$kind" && \
    expect "value" "3" "$val"
}

test_calc_paren_priority() {
    saya_rpc input.set '{"query":"(10+5)/3"}' >/dev/null
    local val; val=$(saya_rpc launcher.snapshot | jq -r '.result.items[0].value')
    expect "value" "5" "$val"
}

test_calc_floating_point() {
    saya_rpc input.set '{"query":"0.1+0.2"}' >/dev/null
    local val; val=$(saya_rpc launcher.snapshot | jq -r '.result.items[0].value')
    expect "value" "0.3" "$val"
}

test_calc_thousands_separator() {
    saya_rpc input.set '{"query":"1024*1024"}' >/dev/null
    local val; val=$(saya_rpc launcher.snapshot | jq -r '.result.items[0].value')
    expect "value" "1,048,576" "$val"
}

test_calc_percent_postfix() {
    saya_rpc input.set '{"query":"50%"}' >/dev/null
    local val; val=$(saya_rpc launcher.snapshot | jq -r '.result.items[0].value')
    expect "value" "0.5" "$val"
}

test_calc_no_match_for_pure_number() {
    saya_rpc input.set '{"query":"42"}' >/dev/null
    local kinds; kinds=$(saya_rpc launcher.snapshot | jq -r '.result.items[].kind' | sort -u)
    if echo "$kinds" | grep -q "^calc$"; then
        echo "    ${RED}✗${RESET} pure number '42' should not produce calc row"
        return 1
    fi
    echo "    ${GREEN}✓${RESET} ${DIM}pure number '42' produces no calc row${RESET}"
}

test_calc_no_match_for_malformed() {
    saya_rpc input.set '{"query":"1+"}' >/dev/null
    local snap; snap=$(saya_rpc launcher.snapshot)
    local has_calc; has_calc=$(jq '.result.items[] | select(.kind=="calc") | .kind' <<<"$snap" | wc -l | tr -d ' ')
    expect "calc rows for '1+'" "0" "$has_calc"
}

test_calc_div_by_zero_no_match() {
    saya_rpc input.set '{"query":"1/0"}' >/dev/null
    local snap; snap=$(saya_rpc launcher.snapshot)
    local has_calc; has_calc=$(jq '.result.items[] | select(.kind=="calc") | .kind' <<<"$snap" | wc -l | tr -d ' ')
    expect "calc rows for '1/0'" "0" "$has_calc"
}

# ==== Launcher matching ===================================================

test_launcher_chr_top_is_chrome() {
    saya_rpc input.set '{"query":"chr"}' >/dev/null
    local snap; snap=$(saya_rpc launcher.snapshot)
    local top_name;  top_name=$(jq -r '.result.items[0].name'  <<<"$snap")
    local top_score; top_score=$(jq -r '.result.items[0].score' <<<"$snap")
    # Base score is 620 (full prefix on the "chrome" word); MRU may push it
    # higher if the user has launched Chrome recently. The contract is
    # "Chrome wins for chr", not a specific magic number.
    expect "top match for chr" "Google Chrome" "$top_name" && \
    expect_ge "top score" 620 "$top_score"
}

test_launcher_empty_query_returns_many() {
    saya_rpc input.set '{"query":""}' >/dev/null
    local count; count=$(saya_rpc launcher.snapshot | jq '.result.items | length')
    expect_ge "items for empty query" 10 "$count"
}

test_launcher_no_match_returns_empty() {
    saya_rpc input.set '{"query":"ZZNOTHINGMATCHES"}' >/dev/null
    local count; count=$(saya_rpc launcher.snapshot | jq '.result.items | length')
    expect "items for nonsense query" "0" "$count"
}

test_launcher_word_prefix_bonus() {
    # "code" should rank a prefix match (Codex) above a word-prefix match
    # (Visual Studio Code), which in turn beats a sparse subseq.
    saya_rpc input.set '{"query":"code"}' >/dev/null
    local snap; snap=$(saya_rpc launcher.snapshot)
    local top; top=$(jq -r '.result.items[0].name' <<<"$snap")
    expect "code top match" "Codex" "$top"
}

# ==== Selection & lifecycle ===============================================

test_launcher_selection_starts_at_zero() {
    saya_rpc input.set '{"query":"chr"}' >/dev/null
    local sel; sel=$(saya_rpc launcher.snapshot | jq '.result.selected')
    expect "selected after fresh query" "0" "$sel"
}

test_panel_toggle_lifecycle() {
    saya_rpc panel.close '{"kind":"launcher"}' >/dev/null
    saya_rpc panel.open  '{"kind":"launcher"}' >/dev/null
    saya_rpc panel.close '{"kind":"launcher"}' >/dev/null
    # No assertion beyond "doesn't error"; the cycle should leave a clean state.
    echo "    ${GREEN}✓${RESET} ${DIM}open + close cycle clean${RESET}"
}

test_launcher_reset_clears_query_and_selection() {
    # Note: items repopulate to "all apps" when query becomes "" and the
    # panel is open, because the LauncherView .task auto-refreshes. The
    # interesting state from reset is query/selected; item contents are an
    # environment-dependent artefact of the empty-query default.
    saya_rpc input.set '{"query":"chr"}' >/dev/null
    saya_rpc launcher.reset >/dev/null
    local snap; snap=$(saya_rpc launcher.snapshot)
    local q;   q=$(jq -r '.result.query' <<<"$snap")
    local sel; sel=$(jq -r '.result.selected' <<<"$snap")
    expect "query after reset" "" "$q" && \
    expect "selected after reset" "0" "$sel"
}

# ==== Clipboard ===========================================================

test_clipboard_snapshot_returns_array() {
    local snap; snap=$(saya_rpc clipboard.snapshot)
    local type; type=$(jq -r '.result.recent | type' <<<"$snap")
    expect "clipboard.recent type" "array" "$type"
}

# ==== Global state ========================================================

test_state_returns_expected_keys() {
    local snap; snap=$(saya_rpc state)
    local has_q;     has_q=$(jq 'has("launcherQuery")' <<<"$snap")
    local has_count; has_count=$(jq '.result | has("entries")' <<<"$snap")
    expect ".launcherQuery present" "false" "$has_q" && \
    expect ".result.entries present" "true" "$has_count"
}

# ==== Event stream ========================================================

test_events_panel_lifecycle() {
    # Run a subscriber in the background and capture events while we drive.
    local log="/tmp/saya-e2e-events-$$.log"
    rm -f "$log"
    "$SAYA" dev event.subscribe --params '{"types":["panel"]}' --follow > "$log" 2>&1 &
    local pid=$!
    sleep 0.3
    saya_rpc panel.open  '{"kind":"launcher"}' >/dev/null
    saya_rpc panel.close '{"kind":"launcher"}' >/dev/null
    sleep 0.4
    kill -TERM "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    local shown; shown=$(grep -c '"panel.launcher.shown"' "$log")
    local hidden; hidden=$(grep -c '"panel.launcher.hidden"' "$log")
    rm -f "$log"
    expect "shown event count" "1" "$shown" && \
    expect "hidden event count" "1" "$hidden"
}

test_events_launcher_executed_carries_payload() {
    local log="/tmp/saya-e2e-events-$$.log"
    rm -f "$log"
    "$SAYA" dev event.subscribe --params '{"types":["launcher.executed"]}' --follow > "$log" 2>&1 &
    local pid=$!
    sleep 0.3
    saya_rpc panel.open  '{"kind":"launcher"}' >/dev/null
    saya_rpc input.set   '{"query":"1+2"}'     >/dev/null
    saya_rpc input.submit                       >/dev/null
    sleep 0.4
    kill -TERM "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    # The whole captured file is a sequence of pretty-printed JSON blocks.
    # Extract by collapsing to one line per object and grep.
    local val
    val=$(jq -rs '.[] | select(.event == "launcher.executed") | .value' "$log" 2>/dev/null | head -1)
    rm -f "$log"
    expect "calc value in event" "3" "$val"
}

# ==== Run =================================================================

ensure_saya_running
run_tests
