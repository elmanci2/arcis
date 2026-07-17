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

use crate::state::Lexer;

/// Lex `source` into a complete token stream (always terminated with
/// `TokenKind::Eof`).
pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    let mut lx = Lexer::new(source);
    let mut tokens = Vec::new();

    loop {
        scanner::skip_whitespace_and_comments(&mut lx);
        if lx.is_eof() {
            tokens.push(Token {
                kind: TokenKind::Eof,
                lexeme: String::new(),
                line: lx.line,
                col: lx.col,
            });
            return Ok(tokens);
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