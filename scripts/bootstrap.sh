#!/usr/bin/env bash
# Bootstrap a fresh checkout: verify the Rust toolchain and ensure the
# workspace at least compiles.

set -euo pipefail

# Resolve the repo root (one level up from this script).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# Verify rustup is installed.
if ! command -v rustup >/dev/null 2>&1; then
    echo "error: rustup not found. Install it from https://rustup.rs/" >&2
    exit 1
fi

echo "→ rustup show"
rustup show

echo "→ cargo check --workspace"
cargo check --workspace --all-targets

echo
echo "✓ workspace bootstrapped successfully."
echo "  next: run \`cargo test --workspace\` to run the test suite."
