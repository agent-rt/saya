-- Drive a launcher scenario. Args:
--   $1: query string to type (default "chr")
--   $2: action — "launch" (Return), "dismiss" (Esc), "leave" (no key) — default "dismiss"
--   $3: delay seconds after typing before the action — default 1.2

on run argv
    set theQuery to "chr"
    set theAction to "dismiss"
    set theDelay to 1.2
    set theAppPath to ""

    if (count of argv) >= 1 then set theQuery to item 1 of argv
    if (count of argv) >= 2 then set theAction to item 2 of argv
    if (count of argv) >= 3 then set theDelay to (item 3 of argv) as real
    if (count of argv) >= 4 then set theAppPath to item 4 of argv

    -- Open the launcher via URL scheme. Far more reliable than synthetic
    -- ⌥ Space, which modern macOS routinely filters before Carbon hotkey
    -- handlers see it. When `theAppPath` is provided, route the URL through
    -- that specific bundle so a brew-installed sibling doesn't steal it.
    if theAppPath is "" then
        do shell script "open 'saya://launcher'"
    else
        do shell script "open -a " & quoted form of theAppPath & " 'saya://launcher'"
    end if
    delay 0.6

    tell application "System Events"
        keystroke theQuery
        delay theDelay
        if theAction is "launch" then
            key code 36 -- Return
        else if theAction is "dismiss" then
            key code 53 -- Esc
        end if
    end tell
end run
