//! Parser state machine.
//!
//! Holds the full token stream and a cursor position. The cursor never
//! wraps — `advance` saturates at the last token so callers can always
//! peek without `Option`-unwrapping, except for [`peek_at`] which is the
//! bounded look-ahead.
//!
//! The various `parse_*` methods live in sibling modules
//! ([`stmt`](super::stmt), [`expr`](super::expr), [`types`](super::types),
//! [`modules`](super::modules)) and attach as additional `impl Parser`
//! blocks.

use arcis_lexer::{Token, TokenKind};

use crate::error::ParseError;

/// Mutable parser state: the full token stream and a cursor position.
pub(crate) struct Parser {
    pub(crate) tokens: Vec<Token>,
    pub(crate) pos: usize,
}

impl Parser {
    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    pub(crate) fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    /// Look-ahead: returns the token at `pos + offset` without consuming.
    pub(crate) fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    pub(crate) fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    /// Returns `true` if the current token has the same variant as `kind`.
    /// Compares discriminants only — payloads (e.g. the literal value of
    /// `Ident`) are ignored.
    pub(crate) fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind)
    }

    /// If the current token matches `kind`, consume it and return `true`.
    /// Otherwise return `false` without consuming.
    pub(crate) fn matches(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consume the current token if it matches `kind`; otherwise emit a
    /// `ParseError` whose `msg` includes the kind and a `context` string.
    pub(crate) fn expect(
        &mut self,
        kind: &TokenKind,
        context: &str,
    ) -> Result<Token, ParseError> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            let t = self.peek();
            Err(ParseError {
                line: t.line,
                col: t.col,
                msg: format!("expected {}, found {}", kind, context),
            })
        }
    }

    /// Consume an identifier token and return its name string.
    pub(crate) fn expect_ident(&mut self, context: &str) -> Result<String, ParseError> {
        let tok = self.expect(&TokenKind::Ident(String::new()), context)?;
        match tok.kind {
            TokenKind::Ident(s) => Ok(s),
            _ => unreachable!(),
        }
    }
}