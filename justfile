# Saya — task runner. Run `just` to list recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Default recipe lists all available tasks.
default:
    @just --list

# Build Saya.app and copy it to ./output/.
# Pass a configuration name as the first arg, e.g. `just build Release`.
build profile="Debug":
    cd apps/saya-macos && xcodegen
    cd apps/saya-macos && xcodebuild \
        -project Saya.xcodeproj \
        -scheme Saya \
        -configuration {{profile}} \
        -derivedDataPath build \
        build
    mkdir -p output
    rm -rf output/Saya.app
    cp -R apps/saya-macos/build/Build/Products/{{profile}}/Saya.app output/Saya.app
    @echo "→ output/Saya.app ({{profile}})"

# Convenience alias for `just build Release`.
release:
    just build Release

# Build and launch the app.
run: build
    open output/Saya.app

# Regenerate Swift bindings from the current FFI surface.
bindgen:
    ./scripts/bindgen.sh

# Regenerate AppIcon.icns from assets/icon.svg.
icon:
    ./scripts/make-icon.sh

# End-to-end dev loop: rebuild, relaunch, drive a query via AppleScript,
# print the log slice produced by this iteration.
#   just devloop                    # default: type "chr", dismiss
#   just devloop "1+2*3"            # custom query
#   just devloop "chr" launch       # type chr, press Return
#   just devloop "" leave 0         # just open the panel, leave it
devloop *args:
    ./scripts/devloop.sh {{args}}

# Tail the Saya log file.
tail-log:
    tail -f -n 50 ~/Library/Logs/Saya/saya.log

# Interactive bug-report capture: restart Saya with debug logging, let user
# reproduce, then writes /tmp/saya-diagnose-<timestamp>.log.
diagnose:
    ./scripts/diagnose.sh

# End-to-end test suite via the DevServer RPC.
# Builds, ensures Saya is running, drives the full state machine through
# JSON-RPC, asserts on snapshots.
e2e:
    just build
    cargo build -p saya-cli
    ./scripts/test/e2e.sh

# Run Rust tests (with the `embedding` feature so the AI path compiles).
test:
    cargo test --features embedding

# Run Rust tests without the embedding feature (faster, no Candle deps).
test-fast:
    cargo test

# Remove every generated artifact.
clean:
    rm -rf output target apps/saya-macos/build apps/saya-macos/Saya.xcodeproj bindings
