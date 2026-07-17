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
- New `sys.*` process-management builtins in a new `sys/process.rs`
  submodule: `process(cmd, args)` (returns the built-in `ArcisProcess
  { stdout: String, stderr: String, exitCode: f64 }` struct),
  `exec(cmd, args?)` (returns stdout as a lossy `string`), `spawn(cmd,
  args?)` (returns the child PID as `number`), `kill(pid)` (sends
  SIGTERM via the `kill` binary), `currentPid()`, `parentPid()` (uses
  `std::os::unix::process::parent_id` on unix, `0` on windows via
  runtime `cfg!`), and `processes()` (parses `ps -e -o pid=,comm=` and
  returns `pid=<n>;name=<s>` per process; empty on windows).
- New built-in struct return type `ArcisProcess`. Codegen
  unconditionally emits a `pub struct ArcisProcess { pub stdout: String,
  pub stderr: String, pub exitCode: f64 }` at the root module (via
  `arcis_ast::Type` injection in `generate_all`), so users can write
  `let p = sys.process(...)` without a type annotation.
- 8 new integration tests for the process builtins (39 sys_codegen tests
  total).
- New `sys.*` sub-namespace call dispatch: `sys.<ns>.<method>(args)`
  routes through a new `crate::sys::emit_subns_call` function which
  delegates to per-namespace `try_emit_method` handlers.
- Six new sub-namespace modules under `crates/arcis-codegen/src/sys/`:
  - `env.rs` — environment variables: `sys.env.get`/`set`/`delete`/`all`.
  - `os.rs` — OS info: `sys.os.name`/`version`/`arch`/`hostname`/
    `username`/`uptime`/`locale`/`cpuCount`. Cross-platform via stdlib
    (`std::env::consts::ARCH`, `available_parallelism`, `cfg!(...)`).
  - `memory.rs` — RAM info: `sys.memory.total`/`free`/`used`/
    `available` (Linux-first via `/proc/meminfo`, returns 0 elsewhere).
  - `cpu.rs` — CPU info: `sys.cpu.model`/`brand`/`frequency`/`usage`/
    `cores`. `usage` samples `/proc/stat` twice with a 100ms sleep.
  - `gpu.rs` — GPU info: `sys.gpu.list`/`name`/`vendor`/`memory`. Linux
    via `lspci -vmm` and `nvidia-smi`; stubs return `[]` or `0` elsewhere.
  - `disk.rs` — disk info: `sys.disk.list`/`free`/`used`/`total`.
    Cross-platform via `df -B1 -P`.
- Renamed `sys/env.rs` → `sys/proc_env.rs` (process queries: currentDir,
  tempDir, homeDir, executablePath, changeDir) so the `sys.env` name
  is free for the new environment-variable namespace.
- New `super::emit_linux_gated` helper in `sys/mod.rs` that emits a
  runtime `cfg!(target_os = "linux")` branch with a default fallback,
  used by every Linux-first builtin.
- 29 new integration tests for the sub-namespace builtins (68
  sys_codegen tests total).
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