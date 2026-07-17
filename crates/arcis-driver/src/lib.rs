//! Arcis driver: orchestrates the compilation pipeline.
//!
//! Phases:
//!   1. Resolve modules from the entry point (by convention `main.tsr`)
//!   2. Run semantic validation per module
//!   3. Run codegen — one `.rs` per module
//!   4. Write to `bin/` (direct rustc) or `bin/<pkg>/` (Cargo project)
//!   5. Invoke `rustc` or `cargo build`/`run` to produce the binary
//!   6. (Optional) Execute the binary
//!
//! ## Layout
//!
//! - [`build`](self::build) — the multi-phase build pipeline.
//! - [`rustc`](self::rustc) — rustc / cargo invocation helpers.
//! - [`init`](self::init) — project scaffolding (`arcis init`).
//!
//! The public entry points ([`build`], [`compile`], [`run`], [`init`]) are
//! thin shims that call into these modules.

use std::path::{Path, PathBuf};

mod build;
mod init;
mod rustc;

/// Result of [`build`]: the generated sources and the layout chosen.
pub struct BuildOutput {
    /// `(id, rust_source)` per module.
    pub sources: Vec<(String, String)>,
    /// The root module id (the entry point).
    pub root_id: String,
    /// When the entry has a `Cargo.toml` beside it, this is the path to the
    /// generated Cargo project. `None` for the direct-rustc path.
    pub cargo_dir: Option<PathBuf>,
    /// When a Cargo project was used, the package name. `None` otherwise.
    pub pkg_name: Option<String>,
}

/// Resolve, validate, generate Rust sources and write them to disk.
/// If the entry has a `Cargo.toml` beside it, generate a Cargo project in
/// `bin/<pkg>/`; otherwise write the `.rs` files directly to `bin/`.
pub fn build(input: &Path) -> Result<BuildOutput, String> {
    build::run(input)
}

/// Compile, then return the path of the produced binary.
pub fn compile(input: &Path) -> Result<PathBuf, String> {
    rustc::compile(input)
}

/// Build, then execute the binary. The exit status of the binary is
/// forwarded to the caller.
pub fn run(input: &Path) -> Result<(), String> {
    rustc::run(input)
}

/// Create a minimal Arcis project in `dir`. See [`init::run`] for details.
pub fn init(dir: &Path) -> Result<(), String> {
    init::run(dir)
}