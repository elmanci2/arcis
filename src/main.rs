//! Entry point del binario `arcis`.
//!
//! Subcomandos:
//!   arcis build [<archivo.tsr> | <dir>]  → compila y deja el binario en ./bin/
//!   arcis run   [<archivo.tsr> | <dir>]  → compila y ejecuta el binario
//!   arcis check [<archivo.tsr> | <dir>]  → solo resuelve+codegen (sin rustc)
//!
//! Sin argumento se usa el directorio actual, donde se busca `main.tsr`
//! (convención de punto de entrada). Si se pasa un directorio se busca
//! `<dir>/main.tsr`; si se pasa un archivo se usa ese.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "arcis", version, about = "Compilador de .tsr a binario nativo", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compila a un binario en ./bin/
    Build {
        /// Ruta al .tsr o directorio (por defecto `.` → busca `main.tsr`).
        file: Option<PathBuf>,
    },
    /// Compila y ejecuta
    Run {
        /// Ruta al .tsr o directorio (por defecto `.` → busca `main.tsr`).
        file: Option<PathBuf>,
    },
    /// Solo genera los .rs (sin invocar rustc) e imprime el de la raíz.
    Check {
        /// Ruta al .tsr o directorio (por defecto `.` → busca `main.tsr`).
        file: Option<PathBuf>,
    },
    /// Inicializa un nuevo proyecto Arcis (crea `main.tsr` con un "Hola")
    Init {
        /// Directorio donde crear el proyecto (por defecto `.`).
        dir: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build { file } => arcis::driver::compile(&resolve_arg(file)).map(|_| ()),
        Commands::Run { file } => arcis::driver::run(&resolve_arg(file)),
        Commands::Check { file } => check_only(&resolve_arg(file)),
        Commands::Init { dir } => arcis::driver::init(&resolve_arg(dir)),
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
    // Sin argumento: directorio actual (se busca `./main.tsr`).
    file.unwrap_or_else(|| PathBuf::from("."))
}

/// Resuelve, valida y genera los `.rs`; imprime el código del módulo raíz.
fn check_only(input: &std::path::Path) -> Result<(), String> {
    let out = arcis::driver::build(input)?;
    println!("{}", out.sources[0].1);
    Ok(())
}
