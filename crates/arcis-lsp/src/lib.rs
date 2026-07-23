//! Arcis Language Server.
//!
//! Provides completion, hover and diagnostics for `.tsr` files via the
//! Language Server Protocol. The LSP binary (`arcis-lsp`) is a thin
//! wrapper that wires [`server::build_router`] into an `async_lsp::MainLoop`
//! running on stdio — see `src/main.rs`.
//!
//! Modules:
//!
//! - [`builtins`] — static table of every completion / hover candidate
//!   (keywords, primitive types, top-level `sys.*`, sub-namespace
//!   methods, array/string methods).
//! - [`completion`] — context-aware completion provider. Reads the text
//!   up to the cursor and figures out whether the user is typing
//!   `sys.`, `sys.<ns>.`, `<expr>.` or just an identifier, then serves
//!   the appropriate slice of the table.
//! - [`hover`] — hover provider. Looks up the identifier under the
//!   cursor and returns its `detail` + `documentation`.
//! - [`definition`] — go-to-definition provider. Resolves an identifier
//!   to its declaration site, same-file or across a module import.
//! - [`symbols`] — shared AST walk that harvests every named
//!   definition in a document (locals, imports, types, enums, …); the
//!   single source of truth `completion`, `hover`, and `definition` all
//!   build on.
//! - [`diagnostics`] — runs the lexer and parser on the whole
//!   document and emits one `Diagnostic` per issue.
//! - [`server`] — Router construction wiring the three providers
//!   above and publishing diagnostics on every change.

pub mod builtins;
pub mod completion;
pub mod definition;
pub mod diagnostics;
pub mod hover;
pub mod server;
pub mod symbols;

/// Re-export of `async_lsp::lsp_types` so other modules can write
/// `lsp::Foo` without needing to know the async-lsp path.
pub use async_lsp::lsp_types as lsp;
