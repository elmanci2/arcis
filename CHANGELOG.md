# Changelog

All notable changes to Arcis are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Project restructured from a single-crate layout into a Cargo workspace with
  one crate per compilation phase: `arcis-ast`, `arcis-lexer`, `arcis-parser`,
  `arcis-validation`, `arcis-linker`, `arcis-codegen`, `arcis-driver`, `arcis`,
  and `arcis-std` (see `docs/architecture.md`).
- Root documentation translated to English (README, CONTRIBUTING, CODE_OF_CONDUCT,
  architecture notes).
- Module-level doc-comments at the top of each phase crate translated to English.

### Added
- Split `sys` codegen into three submodules under
  `crates/arcis-codegen/src/sys/`: `fs` (file/dir IO), `path` (path
  queries) and `env` (process / environment). Each submodule exposes a
  `try_emit(...) -> bool` and the dispatcher (`sys::emit_call`) tries
  each in order before falling back to a verbatim re-emit so `rustc`
  can report unknown `sys.X`.
- New `sys.*` builtins:
  - filesystem: `copy`, `move`, `rename`, `deleteDir`, `deleteDirAll`,
    `createFile`, `readBytes`, `writeBytes`, `appendFile`.
  - path: `isFile`, `isDir`, `fileSize`, `fileInfo` (returns a summary
    string `size=…;is_file=…;is_dir=…;modified_secs=…`), `absolute`,
    `relative`, `createSymlink` (canonicalises the target so
    `sys.exists(link)` resolves correctly), `readLink`.
  - environment: `currentDir`, `changeDir`, `tempDir`, `homeDir`
    (resolves `HOME` / `USERPROFILE` at runtime via `cfg!(windows)`),
    `executablePath`.
- New integration test file `crates/arcis-codegen/tests/sys_codegen.rs`
  covering every `sys.*` builtin (31 tests). The dispatcher fallback
  for unknown `sys.X` is also covered.
- Workspace-level `Cargo.toml` with shared metadata and dependencies.
- `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml` configuration.
- `LICENSE` (MIT), `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`.
- `tests/` integration test scaffolding (`lexer`, `parser`, `codegen`, `linker`).
- `docs/` index, architecture document, language reference, contributing guide.
- `scripts/bootstrap.sh` and `scripts/clean.sh` developer scripts.
- `.github/workflows/ci.yml` running `cargo fmt`, `cargo clippy`, `cargo test`.
- Scaffolding for the `arcis-std` standard library crate (not yet wired in).

### Removed
- Pre-compiled binaries (`bin/main`, `bin/baseline`, `bin/input`) and
  generated `.rs` intermediates (`bin/mate.rs`, `bin/shop.rs`, ...) from
  version control — these are reproducible build artifacts.

## [0.1.0] - 2026-07-17

### Added
- Initial release of Arcis: a TypeScript-like language (`.tsr`) that transpiles
  to Rust and compiles to native binaries via `rustc` or `cargo`.
- CLI subcommands: `build`, `run`, `check`, `init`.
- Multi-module support via DFS module resolution and TypeScript-style
  `import`/`export` syntax.
- Builtins `print`, `input`, `sys.*` (filesystem and `argv` operations).
- Array methods: `find`, `filter`, `map`, `reduce`, `pop`, `push`, `unshift`.
- String methods: `toUpperCase`, `toLowerCase`, `trim`, `substring`,
  `indexOf`, `includes`, `charAt`, `.length`.
- External crate imports via `import { ... } from "crate:<name>";`.

[Unreleased]: https://github.com/elmanci2/arcis/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/elmanci2/arcis/releases/tag/v0.1.0