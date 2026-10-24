default:
    @just --list

# Install locked frontend dependencies.
install:
    npm ci --prefix web

# Build the static UI before compiling the embedded application.
build:
    npm run build --prefix web
    cargo build --locked --release

# Check formatting, TypeScript, and Rust diagnostics.
check:
    npm run format:check --prefix web
    npm run build --prefix web
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    git diff --check

# Exercise authentication, plugin boundaries, stale data, and probes.
test:
    npm run build --prefix web
    cargo test --locked

# Run a loopback-only instance with the generic system plugin.
run:
    npm run build --prefix web
    cargo run --locked -- --config config/example.toml
