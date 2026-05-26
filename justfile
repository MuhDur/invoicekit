# InvoiceKit canonical task runner.
# Usage: `just <recipe>`. Run `just --list` for the full menu.

set shell := ["bash", "-uc"]

# Default: show recipes.
default:
    @just --list

# Build the whole workspace (all crates, all targets).
build:
    cargo build --workspace --all-targets

# Build in release mode.
build-release:
    cargo build --workspace --all-targets --release

# Run the whole test suite.
test:
    cargo test --workspace

# Run unit + doctests on a single crate. Example: `just test-one money`.
test-one crate:
    cargo test -p {{crate}} --all-targets

# Clippy with warnings as errors.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format every file.
fmt:
    cargo fmt --all

# Check formatting without rewriting files. Used by CI.
fmt-check:
    cargo fmt --all --check

# Build all rustdoc, treating warnings as errors.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Run `cargo audit` (requires cargo-audit installed; `cargo install cargo-audit`).
audit:
    cargo audit

# Run `cargo deny check` (requires cargo-deny installed; `cargo install cargo-deny`).
deny:
    cargo deny check bans licenses sources advisories

# Full local CI: format, lint, build, test, doc.
ci: fmt-check lint build test doc

# Clean target/ and incremental build cache.
clean:
    cargo clean
