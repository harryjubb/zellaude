# List available recipes
default:
    @just --list

# Build the WASM plugin
build:
    cargo build --release --target wasm32-wasip1

# Run clippy lints
lint:
    cargo clippy --target wasm32-wasip1 -- -D warnings

# Release a new version (patch, minor, major, or exact version e.g. 1.2.3)
release level:
    cargo release {{level}} --execute
