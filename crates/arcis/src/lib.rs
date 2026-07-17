//! Library API for the `arcis` CLI crate.
//!
//! The binary lives at [`main.rs`](main.rs). This `lib.rs` exists so other
//! Rust crates can depend on the CLI as a library (e.g. integration tests
//! and embedders) without duplicating the command-line plumbing.
//!
//! It is intentionally minimal — the heavy lifting lives in `arcis-driver`.

pub mod cli;

pub use arcis_driver::{build, compile, init, run, BuildOutput};