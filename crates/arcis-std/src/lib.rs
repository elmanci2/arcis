//! Arcis standard library (source-only crate).
//!
//! This crate carries the `.tsr` source files of the Arcis standard library
//! under its package root (`std.tsr`, `strings.tsr`, `arrays.tsr`,
//! `math.tsr`). It compiles to **no Rust code** today — its sole purpose is
//! to ship the `.tsr` files as part of the workspace distribution.
//!
//! The `arcis-driver` crate embeds these files at compile time via
//! `include_str!` so users can `import { trim, sum } from "std";` as if it
//! were a normal module. Wiring that is part of phase 2.
//!
//! Once phase 2 lands, this `lib.rs` may grow to expose Rust-side helpers
//! for the driver (e.g. a `pub fn std_sources() -> &'static [(&'static str, &'static str)]`
//! returning `(module_name, source)` pairs for synthesised `Module` entries).

#![doc = include_str!("../README.md")]
#![allow(dead_code)]
