//! Arcis source formatter.
//!
//! Normalizes `.tsr` source to a single canonical style (Prettier-like):
//! 2-space indentation, semicolons after statements, normalized spacing
//! around operators / keywords / punctuation, and **comments preserved**.
//!
//! The formatter is **token-stream based** (not AST based). The lexer is
//! the only thing that understands comments, and it used to discard them;
//! [`arcis_lexer::lex_with_comments`] now returns them alongside the
//! tokens. We merge the two into one position-ordered stream of [`Item`]s
//! and walk it with a small state machine that decides the separator
//! between each pair of items (none / space / newline) and the current
//! indentation depth.
//!
//! ## Why token-stream and not AST?
//!
//! Preserving comments with an AST formatter requires attaching every
//! comment to an AST node, which in turn needs source spans on every
//! node (today only `let` / `const` / `param` carry positions). That is
//! a large, invasive change. Operating on the token stream keeps comments
//! as first-class tokens so they survive trivially, and the indentation
//! is derived from the bracket structure.
//!
//! ## Limitations (v1)
//!
//! - Long lines are not re-flowed: a 200-character `if` condition stays
//!   on one line.
//! - Object / array literals are kept inline (no multi-line expansion).
//! - Import order is not sorted.
//!
//! See [`format`] for the entry point.

mod emit;
mod stream;

pub use stream::Item;

use arcis_lexer::LexError;

/// Format `source` into canonical Arcis style. Returns the formatted
/// source, or the original [`LexError`] if the input cannot be lexed.
///
/// The result is idempotent: `format(format(x)) == format(x)`.
pub fn format(source: &str) -> Result<String, LexError> {
    let (tokens, comments) = arcis_lexer::lex_with_comments(source)?;
    let items = stream::merge(tokens, comments);
    Ok(emit::run(&items))
}