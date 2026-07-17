# Arcis — top-level developer commands.
#
# Quick reference:
#   make                — show this help
#   make test           — run the entire workspace test suite
#   make test-cli       — run only the CLI integration tests
#   make test-init      — run only the `init` CLI tests
#   make test-build     — run only the `build` CLI tests
#   make test-run       — run only the `run` CLI tests
#   make test-check     — run only the `check` CLI tests
#   make test-one TEST=<substring>
#                       — run a specific test by substring match
#   make smoke          — run the end-to-end smoke tests against examples/
#   make fmt            — apply rustfmt
#   make clippy         — run clippy (deny warnings)
#   make doc            — build the docs (cargo doc --no-deps)
#   make clean          — wipe build artifacts
#
# Notes:
# - Every target is just a thin wrapper over `cargo`. You can do the same
#   thing without `make` if you prefer — see the Cargo aliases in
#   `.cargo/config.toml` for shortcuts (`cargo xtest`, `cargo xrun-cli`,
#   etc.).

# `make` with no arguments prints this help.
.DEFAULT_GOAL := help

help:
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  \033[1m%-20s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# ── Tests ──────────────────────────────────────────────────────────────────

test: ## run the entire workspace test suite
	cargo test --workspace --all-targets

test-one: ## run a specific test by substring match (use TEST=foo)
	cargo test --workspace $(TEST)

test-cli: ## run only the CLI integration tests
	cargo test -p arcis --test cli

test-cli-init: ## run only the `init` CLI tests
	cargo test -p arcis --test cli init

test-cli-build: ## run only the `build` CLI tests
	cargo test -p arcis --test cli build_

test-cli-run: ## run only the `run` CLI tests
	cargo test -p arcis --test cli run_

test-cli-check: ## run only the `check` CLI tests
	cargo test -p arcis --test cli check_

# ── Smoke tests against examples/ ──────────────────────────────────────────

smoke: ## end-to-end smoke test against the examples/ directory
	cargo run --quiet -- run examples/complete/complete.tsr
	@echo "---"
	cargo run --quiet -- run examples/mods

# ── Quality ────────────────────────────────────────────────────────────────

fmt: ## apply rustfmt to the whole workspace
	cargo fmt --all

fmt-check: ## verify formatting without modifying files (CI-friendly)
	cargo fmt --all -- --check

clippy: ## run clippy, treating warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

doc: ## build the rustdoc (no dependencies)
	cargo doc --workspace --no-deps

# ── Maintenance ────────────────────────────────────────────────────────────

clean: ## wipe target/, bin/ contents, and Cargo's incremental caches
	cargo clean
	rm -rf bin/*

.PHONY: help test test-one test-cli test-cli-init test-cli-build test-cli-run test-cli-check smoke fmt fmt-check clippy doc clean