# Contributing to Arcis

Thanks for your interest in improving Arcis! This document covers the workflow
and conventions for contributing to the project.

## Code of conduct

By participating you agree to abide by the [Code of Conduct](CODE_OF_CONDUCT.md).
Please report unacceptable behaviour to the maintainers.

## Getting started

1. Install the Rust toolchain (`rustup`, stable channel — pinned in
   `rust-toolchain.toml`).
2. Clone the repository.
3. Run `./scripts/bootstrap.sh` — it verifies the toolchain and runs
   `cargo check --workspace`.
4. Make your change.
5. Run the full local check: `./scripts/bootstrap.sh` plus `cargo test --workspace`.

## Project structure

Arcis is a Cargo **workspace** with one crate per compilation phase. See
[`docs/architecture.md`](docs/architecture.md) for the dependency graph and
[`docs/README.md`](docs/README.md) for the documentation index.

## Code style

- Run `cargo fmt` before committing; CI runs `cargo fmt --check`.
- Run `cargo clippy --workspace -- -D warnings`; CI runs it on every push.
- Comments and identifiers are in **English**. New code should not introduce
  non-English identifiers, error messages, or doc-comments.
- Public items must have a `///` doc-comment. Internal items should have a
  short comment only when the intent is non-obvious.

## Tests

- Integration tests live under `tests/`, one file per phase crate.
- Inline `#[cfg(test)] mod tests` modules are welcome inside each crate.
- For end-to-end tests, prefer adding an `.tsr` snippet under `examples/` and
  referencing it from `tests/`.

## Adding a builtin

See [`docs/contributing/adding-builtins.md`](docs/contributing/adding-builtins.md).

## Commit messages

- Imperative mood, present tense: "Add X", "Fix Y", not "Added X" or "Fixes Y".
- First line ≤ 72 characters.
- Reference issues in the body when relevant (`Closes #123`, `Refs #456`).
- Sign off with `Co-Authored-By` only when an AI assistant co-authored the work.

## Pull requests

- Keep changes focused; one logical change per PR.
- Update `CHANGELOG.md` under `[Unreleased]` for user-visible changes.
- Update or add tests.
- Make sure `cargo fmt`, `cargo clippy`, and `cargo test` all pass locally
  before opening the PR.

## Releasing

1. Bump `version` in `[workspace.package]` of `Cargo.toml`.
2. Move the `[Unreleased]` section of `CHANGELOG.md` into a dated version
   section.
3. Tag the release (`git tag -s vX.Y.Z -m "vX.Y.Z"`).
4. Push the tag; CI publishes the release.

## Questions?

Open an issue. We are happy to help.