//! Crate `arcis`: un lenguaje con sintaxis TypeScript-like que compila a
//! binario nativo transpilando primero a Rust y delegando en `rustc`.
//!
//! Pipeline: `lexer` → `parser` → `codegen` → `driver` (→ `rustc`).
//!
//! Esta crate expone los módulos públicos; el binario vive en `main.rs`.

pub mod ast;
pub mod codegen;
pub mod driver;
pub mod lexer;
pub mod modules;
pub mod parser;
pub mod token;
pub mod validation;