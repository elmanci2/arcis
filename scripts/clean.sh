#!/usr/bin/env bash
# Wipe all transient build artifacts. Safe to run anywhere in the repo.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

echo "→ cargo clean"
cargo clean

# The legacy direct-rustc pipeline used to write to bin/. After cargo clean
# we also remove any leftover .rs files or compiled binaries there.
if [ -d bin ]; then
    echo "→ wiping bin/ contents"
    rm -rf bin/*
fi

echo
echo "✓ build artifacts removed."
