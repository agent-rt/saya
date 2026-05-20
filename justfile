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

# Run Rust tests (with the `embedding` feature so the AI path compiles).
test:
    cargo test --features embedding

# Run Rust tests without the embedding feature (faster, no Candle deps).
test-fast:
    cargo test

# Remove every generated artifact.
clean:
    rm -rf output target apps/saya-macos/build apps/saya-macos/Saya.xcodeproj bindings
