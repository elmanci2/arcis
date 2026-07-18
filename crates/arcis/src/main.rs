//! Entry point of the `arcis` binary.
//!
//! Subcommands:
//!   arcis build [--backend rust|cranelift] [<file.tsr> | <dir>]  → compile
//!   arcis run   [--backend rust|cranelift] [<file.tsr> | <dir>]  → compile and execute
//!   arcis check [--backend rust|cranelift] [<file.tsr> | <dir>]  → emit without compiling
//!   arcis init  [<dir>]                                          → scaffold a new project
//!
//! Without an argument the current directory is used, where the driver looks
//! for `main.tsr` (the entry-point convention). With a directory it looks for
//! `<dir>/main.tsr`; with a file it uses that file directly.
//!
//! ## Backends
//!
//! - `rust` (default): lower to Rust source, compile with `rustc`/`cargo`.
//!   Requires a Rust toolchain installed.
//! - `cranelift`: lower directly to Cranelift IR, link with the system `cc`
//!   against `libc`. **Does not** require a Rust toolchain.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "arcis",
    version,
    about = "Arcis compiler: turn .tsr into native binaries",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile to a binary under ./bin/
    Build {
        /// Path to the .tsr file or directory (default `.` → searches `main.tsr`).
        file: Option<PathBuf>,
        /// Codegen backend: `rust` (default, needs rustc) or `cranelift` (no rustc needed).
        #[arg(long, value_name = "BACKEND", default_value = "rust")]
        backend: String,
    },
    /// Compile and execute the binary
    Run {
        /// Path to the .tsr file or directory (default `.` → searches `main.tsr`).
        file: Option<PathBuf>,
        /// Codegen backend: `rust` (default, needs rustc) or `cranelift` (no rustc needed).
        #[arg(long, value_name = "BACKEND", default_value = "rust")]
        backend: String,
    },
    /// Only emit the source files (without invoking the final compiler) and
    /// print the root module's source.
    Check {
        /// Path to the .tsr file or directory (default `.` → searches `main.tsr`).
        file: Option<PathBuf>,
        /// Codegen backend: `rust` (default) or `cranelift` (dumps Cranelift IR).
        #[arg(long, value_name = "BACKEND", default_value = "rust")]
        backend: String,
    },
    /// Initialise a new Arcis project (creates `main.tsr` with a "Hello"
    /// example ready to run).
    Init {
        /// Directory in which to create the project (default `.`).
        dir: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build { file, backend } => {
            let backend = match arcis_driver::Backend::parse(&backend) {
                Ok(b) => b,
                Err(e) => return error_exit(e),
            };
            arcis_driver::compile(&resolve_arg(file), backend).map(|_| ())
        }
        Commands::Run { file, backend } => {
            let backend = match arcis_driver::Backend::parse(&backend) {
                Ok(b) => b,
                Err(e) => return error_exit(e),
            };
            arcis_driver::run(&resolve_arg(file), backend)
        }
        Commands::Check { file, backend } => {
            let backend = match arcis_driver::Backend::parse(&backend) {
                Ok(b) => b,
                Err(e) => return error_exit(e),
            };
            check_only(&resolve_arg(file), backend)
        }
        Commands::Init { dir } => arcis_driver::init(&resolve_arg(dir)),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => error_exit(msg),
    }
}

fn resolve_arg(file: Option<PathBuf>) -> PathBuf {
    // No argument: current directory (the driver will search for `./main.tsr`).
    file.unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve, validate and emit the source files; print the root module's code
/// (Rust backend only — the Cranelift backend writes `.o` files which are
/// not human-readable, so we print a one-line summary instead).
fn check_only(input: &std::path::Path, backend: arcis_driver::Backend) -> Result<(), String> {
    let out = arcis_driver::build(input, backend)?;
    match out.backend {
        arcis_driver::Backend::Rust => {
            println!("{}", out.sources[0].1);
        }
        arcis_driver::Backend::Cranelift => {
            println!(
                "(Cranelift backend) — wrote {} object files to `bin/`",
                out.sources.len()
            );
            println!("use `arcis build --backend cranelift {}` to produce a binary", input.display());
        }
    }
    Ok(())
}

fn error_exit(msg: String) -> ExitCode {
    eprintln!("{}", msg);
    ExitCode::FAILURE
}
