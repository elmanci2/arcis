//! Entry point of the `arcis` binary.
//!
//! Subcommands:
//!   arcis build [<file.tsr> | <dir>]  → compile and leave the binary at ./bin/
//!   arcis run   [<file.tsr> | <dir>]  → compile and execute the binary
//!   arcis check [<file.tsr> | <dir>]  → only resolve + codegen (no rustc)
//!   arcis init  [<dir>]               → scaffold a new project
//!
//! Without an argument the current directory is used, where the driver looks
//! for `main.tsr` (the entry-point convention). With a directory it looks for
//! `<dir>/main.tsr`; with a file it uses that file directly.

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
    },
    /// Compile and execute the binary
    Run {
        /// Path to the .tsr file or directory (default `.` → searches `main.tsr`).
        file: Option<PathBuf>,
    },
    /// Only emit the `.rs` files (without invoking rustc) and print the root
    /// module's code.
    Check {
        /// Path to the .tsr file or directory (default `.` → searches `main.tsr`).
        file: Option<PathBuf>,
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
        Commands::Build { file } => arcis_driver::compile(&resolve_arg(file)).map(|_| ()),
        Commands::Run { file } => arcis_driver::run(&resolve_arg(file)),
        Commands::Check { file } => check_only(&resolve_arg(file)),
        Commands::Init { dir } => arcis_driver::init(&resolve_arg(dir)),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{}", msg);
            ExitCode::FAILURE
        }
    }
}

fn resolve_arg(file: Option<PathBuf>) -> PathBuf {
    // No argument: current directory (the driver will search for `./main.tsr`).
    file.unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve, validate and emit the `.rs` files; print the root module's code.
fn check_only(input: &std::path::Path) -> Result<(), String> {
    let out = arcis_driver::build(input)?;
    println!("{}", out.sources[0].1);
    Ok(())
}