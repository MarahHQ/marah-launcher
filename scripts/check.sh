#!/usr/bin/env bash
# The one check: format, lint and test everything. make, the git hooks and CI all run it, so a
# check that passes here passes there.
#   scripts/check.sh          all of it
#   scripts/check.sh lint     the format check and clippy
#   scripts/check.sh fmt      the format check
#   scripts/check.sh clippy   clippy, warnings as errors
#   scripts/check.sh test     the tests
# Cargo runs with --locked: a Cargo.lock that doesn't match Cargo.toml fails here, as in CI.
# After you change a dependency, `cargo update -w` brings the lock up to date.
set -euo pipefail
cd "$(dirname "$0")/.."

part=${1:-all}
case "$part" in all | lint | fmt | clippy | test) ;; *) echo "usage: $0 [lint|fmt|clippy|test]" >&2; exit 2 ;; esac

if [ "$part" = all ] || [ "$part" = lint ] || [ "$part" = fmt ]; then
    cargo fmt --all --check
fi

if [ "$part" = all ] || [ "$part" = lint ] || [ "$part" = clippy ]; then
    cargo clippy --workspace --all-targets --locked -- -D warnings
fi

if [ "$part" = all ] || [ "$part" = test ]; then
    # A failing test binary doesn't hide the failures of the ones after it.
    if cargo nextest --version >/dev/null 2>&1; then
        # Every test in its own process, all of them at once (.config/nextest.toml). nextest
        # leaves out doc tests; the code has none that run.
        cargo nextest run --workspace --locked --no-fail-fast
    else
        cargo test --workspace --locked --no-fail-fast
    fi
fi
