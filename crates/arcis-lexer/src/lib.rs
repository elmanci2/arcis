//! Arcis lexer: source text → token stream.
//!
//! Hand-rolled recursive-style scanner — a single pass over the characters
//! with single-character lookahead (`peek_char`).
//!
//! Supports:
//! - Line (`//`) and block (`/* ... */`) comments
//! - Double-quoted strings with escape sequences (`\n`, `\t`, `\\`, `\"`)
//! - Integer and decimal numbers
//! - Identifiers and TypeScript-style keywords
//!
//! ## Layout
//!
//! - [`state`](self::state) — the `Lexer` state machine (cursor, peek/advance)
//! - [`scanner`](self::scanner) — per-token-type readers + the keyword table
//! - [`error`](self::error) — `LexError` type + `Display` / `Error` impls
//! - [`token`](self::token) — `Token` and `TokenKind` definitions

mod error;
mod scanner;
mod state;
mod token;

// Re-export the public types so downstream crates can write
// `use arcis_lexer::{Token, TokenKind};` directly.
pub use crate::token::{Token, TokenKind};

pub use crate::error::LexError;
pub use crate::scanner::CommentToken;

use crate::state::Lexer;

/// Lex `source` into a complete token stream (always terminated with
/// `TokenKind::Eof`) **and** the list of comments encountered, in source
/// order. The parser ignores comments; this variant exists so the
/// formatter (and other tools that care about comments) can preserve
/// them.
pub fn lex_with_comments(
    source: &str,
) -> Result<(Vec<Token>, Vec<CommentToken>), LexError> {
    let mut lx = Lexer::new(source);
    let mut tokens = Vec::new();
    let mut comments = Vec::new();

    loop {
        scanner::skip_whitespace_and_comments(&mut lx, &mut comments);
        if lx.is_eof() {
            tokens.push(Token {
                kind: TokenKind::Eof,
                lexeme: String::new(),
                line: lx.line,
                col: lx.col,
            });
            return Ok((tokens, comments));
        }

        let line = lx.line;
        let col = lx.col;
        let c = lx.peek_char().unwrap();

        let kind = if scanner::is_ident_start(c) {
            scanner::read_ident(&mut lx)?
        } else if c.is_ascii_digit() {
            scanner::read_number(&mut lx)?
        } else if c == '"' {
            scanner::read_string(&mut lx)?
        } else {
            scanner::read_punct_or_op(&mut lx)?
        };

        let lexeme = lx.slice_current();
        tokens.push(Token { kind, lexeme, line, col });
    }
}

/// Lex `source` into a complete token stream (always terminated with
/// `TokenKind::Eof`). Comments are discarded — use [`lex_with_comments`]
/// if you need them.
pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    let (tokens, _comments) = lex_with_comments(source)?;
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_number_and_identifier() {
        let toks = lex("let x = 42;").expect("lex must succeed");
        let kinds: Vec<&TokenKind> = toks.iter().map(|t| &t.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Let));
        assert!(matches!(kinds[1], TokenKind::Ident(_)));
        assert!(matches!(kinds[2], TokenKind::Eq));
        assert!(matches!(kinds[3], TokenKind::Number(_)));
        assert!(matches!(kinds[4], TokenKind::Semi));
        assert!(matches!(kinds[5], TokenKind::Eof));
    }

    #[test]
    fn unterminated_string_is_an_error() {
        assert!(lex("let s = \"abc").is_err());
    }
}