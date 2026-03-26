# List available recipes
default:
    @just --list

# Build the WASM plugin
build:
    cargo build --release --target wasm32-wasip1

plugin_dir := env("ZELLIJ_PLUGIN_DIR", "~/.config/zellij/plugins")

# Build and install the WASM plugin locally for testing
install: build
    mkdir -p {{plugin_dir}}
    cp target/wasm32-wasip1/release/zellaude.wasm {{plugin_dir}}/zellaude.wasm

# Run clippy lints
lint:
    cargo clippy --target wasm32-wasip1 -- -D warnings

# Release a new version (patch, minor, major, or exact version e.g. 1.2.3)
release level:
    cargo release {{level}} --execute
