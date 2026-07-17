//! Expression parsing.
//!
//! Implements **precedence climbing** for binary operators (lowest to
//! highest precedence):
//!
//! 1. `or`         (`||`)
//! 2. `and`        (`&&`)
//! 3. `equality`   (`==`, `!=`)
//! 4. `comparison` (`<`, `>`, `<=`, `>=`)
//! 5. `additive`   (`+`, `-`)
//! 6. `multiplicative` (`*`, `/`, `%`)
//! 7. `unary`      (`!`, `-`)
//! 8. `postfix`    (`.ident`, `[expr]`, `(args)` chains)
//! 9. `atom`       (literals, identifiers, paths, calls, groups, array/object literals)
//!
//! Each level calls the level below until it reaches an atom.

use arcis_ast::{BinOp, Expr, UnaryOp};
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

impl Parser {
    /// Top-level expression entry point.
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Binary {
                op: BinOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::And) {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::Binary {
                op: BinOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::EqEq => BinOp::EqEq,
                TokenKind::NotEq => BinOp::NotEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::LtEq => BinOp::LtEq,
                TokenKind::GtEq => BinOp::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Bang => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                })
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    /// Parse a postfix chain: starts from an atom and consumes
    /// `.ident`, `[expr]`, and `(args)` repeatedly to build
    /// `Member` / `Index` / `Call` nodes.
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_atom()?;
        loop {
            match self.peek_kind() {
                TokenKind::Dot => {
                    self.advance();
                    let prop_tok = self.expect(
                        &TokenKind::Ident(String::new()),
                        "property name after `.`",
                    )?;
                    let property = match &prop_tok.kind {
                        TokenKind::Ident(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    expr = Expr::Member {
                        object: Box::new(expr),
                        property,
                    };
                }
                TokenKind::LBracket => {
                    self.advance(); // [
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket, "`]` after index expression")?;
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                TokenKind::LParen => {
                    // Calls on any expression: `obj.method(args)`, `f(args)`, etc.
                    self.advance(); // (
                    let mut args = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.matches(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)` after arguments")?;
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        let t = self.advance();
        match t.kind {
            TokenKind::Number(n) => Ok(Expr::Number(n)),
            TokenKind::String(s) => Ok(Expr::String(s)),
            TokenKind::Bool(b) => Ok(Expr::Bool(b)),
            TokenKind::Ident(name) => {
                // Static path: `crate::Type::method`. Only valid at the start
                // of an expression (not chained off another expression).
                if self.check(&TokenKind::ColonColon) {
                    let mut segments = vec![name];
                    while self.matches(&TokenKind::ColonColon) {
                        let next = self.expect(
                            &TokenKind::Ident(String::new()),
                            "path segment after `::`",
                        )?;
                        let segment = match next.kind {
                            TokenKind::Ident(s) => s,
                            _ => unreachable!(),
                        };
                        segments.push(segment);
                    }
                    if self.check(&TokenKind::LParen) {
                        self.advance();
                        let mut args = Vec::new();
                        if !self.check(&TokenKind::RParen) {
                            loop {
                                args.push(self.parse_expr()?);
                                if !self.matches(&TokenKind::Comma) {
                                    break;
                                }
                            }
                        }
                        self.expect(&TokenKind::RParen, "`)` after arguments")?;
                        return Ok(Expr::Call {
                            callee: Box::new(Expr::Path { segments }),
                            args,
                        });
                    }
                    return Ok(Expr::Path { segments });
                }
                if self.check(&TokenKind::LParen) {
                    self.advance(); // (
                    let mut args = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.matches(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)` after arguments")?;
                    Ok(Expr::Call {
                        callee: Box::new(Expr::Ident(name)),
                        args,
                    })
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            TokenKind::LParen => {
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "`)` after grouped expression")?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                // Array literal: `[expr, expr, ...]` or `[]`.
                // The `[` was already consumed by `self.advance()` above.
                let mut elements = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    loop {
                        elements.push(self.parse_expr()?);
                        if !self.matches(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RBracket, "`]` closing array literal")?;
                Ok(Expr::ArrayLiteral { elements })
            }
            TokenKind::LBrace => {
                // Object literal: `{ key: expr, ... }`.
                // Must be in a context with a declared type (let/const); the
                // codegen infers the type from that context.
                let mut fields = Vec::new();
                if !self.check(&TokenKind::RBrace) {
                    loop {
                        let key_tok = self.expect(
                            &TokenKind::Ident(String::new()),
                            "field name in object literal",
                        )?;
                        let key = match &key_tok.kind {
                            TokenKind::Ident(s) => s.clone(),
                            _ => unreachable!(),
                        };
                        self.expect(&TokenKind::Colon, "`:` after field name")?;
                        let value = self.parse_expr()?;
                        fields.push((key, value));
                        if !self.matches(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}` closing object literal")?;
                Ok(Expr::ObjectLiteral { fields })
            }
            other => Err(ParseError {
                line: t.line,
                col: t.col,
                msg: format!("expected an expression, found {}", other),
            }),
        }
    }

    /// Consume a string-literal token and return its contents.
    pub(crate) fn expect_string_literal(&mut self, context: &str) -> Result<String, ParseError> {
        let t = self.peek().clone();
        if let TokenKind::String(s) = &t.kind {
            let s = s.clone();
            self.advance();
            Ok(s)
        } else {
            Err(ParseError {
                line: t.line,
                col: t.col,
                msg: format!("expected a string ({})", context),
            })
        }
    }
}