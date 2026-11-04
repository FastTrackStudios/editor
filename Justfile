# editor — root workspace recipes. Run `just` (no args) for the menu.
# Requires `just` — pre-installed in the nix shell.

# Default: list recipes
default:
    @just --list

# ── Rust ─────────────────────────────────────────────────────────────
# (`check` — native + wasm — is defined once, further down.)

# nextest: parallel per-test binaries. Does NOT run doctests. Then the
# native (Blitz) renderer's tests, a feature the default build leaves out.
rust-test:
    cargo nextest run --workspace
    cargo nextest run -p editor-view --no-default-features --features native

# pedantic + nursery + the no-panic set (Cargo.toml [workspace.lints]), and
# the native renderer's library. Its integration tests (features/ui/tests/
# native_*.rs) are not clippy-clean yet.
clippy:
    cargo clippy --workspace --all-targets
    cargo clippy -p editor-view --no-default-features --features native --lib
    cargo clippy -p editor-view --no-default-features --features native --lib --profile test

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

# The gate CI runs, in CI's order.
ci: fmt-check check clippy rust-test

# The dogfooding app — the fastest way to see a change.
play *ARGS:
    cargo run -p playground {{ARGS}}

# ── Browser tests (playwright, against the playground) ───────────────
# Run the full playwright suite (headless Chromium).
test:
    cd tests/browser && pnpm install --silent && pnpm test

# Run with a visible browser window — debug-mode.
test-headed:
    cd tests/browser && pnpm install --silent && pnpm test:headed

# Open Playwright's interactive UI runner.
test-ui:
    cd tests/browser && pnpm install --silent && pnpm test:ui

# Run only one test by name fragment, e.g.: `just test-only "cursor stays"`.
test-only PATTERN:
    cd tests/browser && pnpm install --silent && npx playwright test -g "{{PATTERN}}"

# Re-run a previously failed test with its trace open.
trace:
    cd tests/browser && npx playwright show-trace test-results/*/trace.zip

# Workspace-wide Rust tests.
unit:
    cargo test --workspace

# Cargo check on every target we care about.
# The wasm check builds the playground the way `dx serve --platform web`
# does: its default feature is `desktop`, whose tokio/getrandom stack does
# not compile for wasm32.
check:
    cargo check --workspace
    cargo check -p playground --target wasm32-unknown-unknown --no-default-features --features web

# Start the desktop playground in dev mode.
dev:
    dx serve

# Start the web playground (matches what playwright uses).
dev-web:
    dx serve --platform web --port 8090
