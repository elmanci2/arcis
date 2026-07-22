//! Arcis driver: orchestrates the compilation pipeline.
//!
//! Phases:
//!   1. Resolve modules from the entry point (by convention `main.tsr`)
//!   2. Run semantic validation per module
//!   3. Run codegen — one `.rs` per module (Rust backend) or one `.o` per
//!      module (Cranelift backend)
//!   4. Write generated files to `bin/` (or `bin/<pkg>/` for Cargo layout,
//!      Rust backend only)
//!   5. Invoke `rustc`/`cargo` (Rust backend) or `cc` (Cranelift backend)
//!      to produce the binary
//!   6. (Optional) Execute the binary
//!
//! ## Layout
//!
//! - [`build`] — the multi-phase build pipeline.
//! - [`rustc`] — rustc / cargo invocation helpers (Rust backend).
//! - [`cranelift`] — `cc` invocation helpers (Cranelift backend).
//! - [`init`] — project scaffolding (`arcis init`).
//!
//! The public entry points ([`build`], [`compile`], [`run`], [`init`]) are
//! thin shims that call into these modules.

use std::path::{Path, PathBuf};

mod build;
mod cranelift;
mod init;
mod rustc;

/// Which codegen backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Lower AST to Rust source, then invoke `rustc`/`cargo`.
    Rust,
    /// Default: lower AST directly to Cranelift IR; link via `cc` against the
    /// bundled C runtime. Does **not** depend on `rustc` being installed.
    Cranelift,
}

impl Backend {
    /// Parse a backend name from the CLI. Case-insensitive.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "rust" => Ok(Backend::Rust),
            "cranelift" | "native" => Ok(Backend::Cranelift),
            other => Err(format!(
                "unknown backend `{}` (expected `rust` or `cranelift`)",
                other
            )),
        }
    }

    /// String form for error messages / display.
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Rust => "rust",
            Backend::Cranelift => "cranelift",
        }
    }
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Cranelift
    }
}

/// Result of [`build`]: the generated sources and the layout chosen.
/// Only meaningful for the Rust backend; the Cranelift backend writes
/// directly to disk and returns a stripped-down output.
pub struct BuildOutput {
    /// `(id, rust_source)` per module (Rust backend only).
    pub sources: Vec<(String, String)>,
    /// The root module id (the entry point).
    pub root_id: String,
    /// When the entry has a `Cargo.toml` beside it (Rust backend only),
    /// this is the path to the generated Cargo project. `None` for the
    /// direct-rustc path.
    pub cargo_dir: Option<PathBuf>,
    /// When a Cargo project was used, the package name. `None` otherwise.
    pub pkg_name: Option<String>,
    /// The backend that produced this output.
    pub backend: Backend,
}

/// Resolve, validate, generate sources (Rust or Cranelift) and write them
/// to disk.
///
/// The exact on-disk layout depends on `backend`:
/// - `Rust`: writes `.rs` files into `bin/` (direct rustc layout) or
///   `bin/<pkg>/` (Cargo layout).
/// - `Cranelift`: writes `.o` files into `bin/` plus `arcis_runtime.o`.
pub fn build(input: &Path, backend: Backend) -> Result<BuildOutput, String> {
    build::run(input, backend)
}

/// Compile, then return the path of the produced binary.
pub fn compile(input: &Path, backend: Backend) -> Result<PathBuf, String> {
    match backend {
        Backend::Rust => rustc::compile(input),
        Backend::Cranelift => cranelift::compile(input),
    }
}

/// Build, then execute the binary. The exit status of the binary is
/// forwarded to the caller.
pub fn run(input: &Path, backend: Backend) -> Result<(), String> {
    match backend {
        Backend::Rust => rustc::run(input),
        Backend::Cranelift => cranelift::run(input),
    }
}

/// Create a minimal Arcis project in `dir`. See [`init::run`] for details.
pub fn init(dir: &Path) -> Result<(), String> {
    init::run(dir)
}