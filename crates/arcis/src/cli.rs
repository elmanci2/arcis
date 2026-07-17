//! Re-exported CLI types for programmatic use.
//!
//! Lets consumers (typically integration tests) build an [`ArgMatches`] by
//! hand without going through the `clap` parsing layer twice.

pub use clap::ArgMatches;
