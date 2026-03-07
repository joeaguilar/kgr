# kgr — Polyglot source dependency knowledge graph
# Run `just` or `just --list` to see all recipes

set dotenv-load

default:
    @just --list

# ─── Build ───────────────────────────────────────────────────────────

# Debug build
build:
    cargo build

# Optimized release build
release:
    cargo build --release

# Check without producing binaries (faster)
check:
    cargo check --workspace

# ─── Test ────────────────────────────────────────────────────────────

# Run all tests
test:
    cargo test --workspace

# Run integration tests only
test-integration:
    cargo test --package kgr --test integration

# Run snapshot tests only
test-snapshots:
    cargo test --package kgr --test snapshots

# Update snapshots after intentional output changes
update-snapshots:
    INSTA_UPDATE=new cargo test --package kgr --test snapshots
    cargo insta accept

# ─── Lint & Format ──────────────────────────────────────────────────

# Run clippy (all targets)
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check

# Full verification: check + test + clippy + format check
verify: check lint test fmt-check

# CI pipeline
ci: fmt-check lint test

# ─── Dogfood — run kgr on itself ────────────────────────────────────

# Show the dependency tree of kgr's own crates
graph:
    cargo run --release -q -- graph --format tree --no-progress crates

# Show table view of kgr's own crates
table:
    cargo run --release -q -- graph --format table --no-progress crates

# Run check on kgr's own crates
kgr-check:
    cargo run --release -q -- check --no-progress crates

# ─── Clean ───────────────────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean

# ─── Issue Tracker (itr) ────────────────────────────────────────────

# Show next actionable task
next:
    itr ready -f json

# List all open issues
issues:
    itr list

# Add a new issue (usage: just issue "title")
issue title:
    itr add "{{title}}"

# Close an issue (usage: just close 3 "reason")
close id reason:
    itr close {{id}} "{{reason}}"

# Add a note to an issue (usage: just note 3 "summary")
note id summary:
    itr note {{id}} "{{summary}}"

# ─── Info ────────────────────────────────────────────────────────────

# Show workspace dependency tree (depth 1)
deps:
    cargo tree --workspace --depth 1

# Show release binary size
size: release
    ls -lh target/release/kgr

# Show lines of code per file
loc:
    @echo "── Source lines ──"
    @find crates -name '*.rs' | xargs wc -l | sort -n
