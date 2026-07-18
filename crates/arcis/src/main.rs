//! Entry point of the `arcis` binary.
//!
//! Subcommands:
//!   arcis build [--backend rust|cranelift] [<file.tsr> | <dir>]  → compile
//!   arcis run   [--backend rust|cranelift] [<file.tsr> | <dir>]  → compile and execute
//!   arcis check [--backend rust|cranelift] [<file.tsr> | <dir>]  → emit without compiling
//!   arcis init  [<dir>]                                          → scaffold a new project
//!   arcis fmt   [--check] [<file.tsr> ...]                       → format .tsr files
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

use std::fs;
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
    /// Format .tsr files (Prettier-style: 2-space indent, semicolons, normalised
    /// spacing).  With --check, exit with a non-zero code if any file *differs*
    /// from the expected formatting — useful for CI.
    Fmt {
        /// .tsr files or directories to format.  Directories are walked
        /// recursively for `**/*.tsr`.
        files: Vec<PathBuf>,
        /// Only check — do not write files.  Exit 1 if formatting would differ.
        #[arg(long, default_value_t = false)]
        check: bool,
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
        Commands::Fmt { files, check } => cmd_fmt(files, check),
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

/// Walk `paths`, formatting every `.tsr` file found.  Directories are
/// walked recursively.  If `check` is true, files are *not* modified and
/// the command exits 1 if any file would differ; otherwise formatted
/// sources are written back in place.
fn cmd_fmt(files: Vec<PathBuf>, check: bool) -> Result<(), String> {
    let mut tsr_files = Vec::new();
    for p in &files {
        if p.is_dir() {
            // Recursively collect .tsr files.
            collect_tsr_files(p, &mut tsr_files)
                .map_err(|e| format!("{}: {e}", p.display()))?;
        } else {
            tsr_files.push(p.clone());
        }
    }
    // Default: current directory.
    if tsr_files.is_empty() {
        collect_tsr_files(&PathBuf::from("."), &mut tsr_files)
            .map_err(|e| format!(". : {e}"))?;
    }

    let mut dirty = 0;
    for path in &tsr_files {
        let src = fs::read_to_string(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        match arcis_fmt::format(&src) {
            Ok(formatted) => {
                if formatted == src {
                    println!("unchanged: {}", path.display());
                } else if check {
                    eprintln!("would format: {}", path.display());
                    dirty += 1;
                } else {
                    fs::write(path, formatted)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    println!("formatted:  {}", path.display());
                }
            }
            Err(e) => {
                eprintln!("{} {}", path.display(), e);
                dirty += 1;
            }
        }
    }

    if dirty > 0 {
        Err(format!(
            "{dirty} file{} {} not formatted",
            if dirty == 1 { "" } else { "s" },
            if check { "would be" } else { "could not be" },
        ))
    } else {
        Ok(())
    }
}

fn collect_tsr_files(dir: &PathBuf, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_tsr_files(&path, out)?;
        } else if path.extension().map(|e| e == "tsr").unwrap_or(false) {
            out.push(path);
        }
    }
    Ok(())
}

fn error_exit(msg: String) -> ExitCode {
    eprintln!("{}", msg);
    ExitCode::FAILURE
}
