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

use arcis_ast::{ArrayElement, ArrowBody, BinOp, Expr, ObjectField, Param, UnaryOp};
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

impl Parser {
    /// Attempt to parse an arrow-function parameter list and body,
    /// starting right after an already-consumed `(`. Returns `Ok(None)`
    /// (never an `Err`) on any syntactic mismatch — including a garbled
    /// partial match — so the caller can cleanly roll back and retry as a
    /// grouped expression instead. Parameters require an explicit type
    /// annotation, same as `function` declarations (Arcis has no
    /// parameter-type inference); this is also what disambiguates
    /// `(x: number) => ...` from a grouped expression `(x)`.
    fn try_parse_arrow_after_lparen(&mut self) -> Result<Option<Expr>, ParseError> {
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let pname_tok = match self.peek_kind() {
                    TokenKind::Ident(_) => self.advance(),
                    _ => return Ok(None),
                };
                let pname = match &pname_tok.kind {
                    TokenKind::Ident(s) => s.clone(),
                    _ => unreachable!(),
                };
                if !self.check(&TokenKind::Colon) {
                    return Ok(None);
                }
                self.advance(); // :
                let Ok(pty) = self.parse_type() else { return Ok(None) };
                params.push(Param { name: pname, ty: pty, line: pname_tok.line, col: pname_tok.col });
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.check(&TokenKind::RParen) {
            return Ok(None);
        }
        self.advance(); // )
        let return_type = if self.matches(&TokenKind::Colon) {
            let Ok(ty) = self.parse_type() else { return Ok(None) };
            Some(ty)
        } else {
            None
        };
        if !self.check(&TokenKind::FatArrow) {
            return Ok(None);
        }
        self.advance(); // =>
        let body = if self.check(&TokenKind::LBrace) {
            self.advance(); // {
            let mut stmts = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
                stmts.push(self.parse_stmt()?);
            }
            self.expect(&TokenKind::RBrace, "`}` closing arrow function body")?;
            ArrowBody::Block(stmts)
        } else {
            ArrowBody::Expr(Box::new(self.parse_expr()?))
        };
        Ok(Some(Expr::Arrow { params, return_type, body }))
    }

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
            TokenKind::Typeof => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::TypeOf(Box::new(operand)))
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
                // Non-null assertion: `expr!`. Distinct from prefix `!`
                // (logical not), which is only ever consumed in
                // `parse_unary` before we get here.
                TokenKind::Bang => {
                    self.advance();
                    expr = Expr::NonNullAssertion(Box::new(expr));
                }
                // Type assertion: `expr as Type` or the const assertion
                // `expr as const`.
                TokenKind::As => {
                    self.advance();
                    if self.check(&TokenKind::Const) {
                        self.advance();
                        expr = Expr::AsConst(Box::new(expr));
                    } else {
                        let ty = self.parse_type()?;
                        expr = Expr::AsAssertion {
                            expr: Box::new(expr),
                            ty,
                        };
                    }
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
            TokenKind::Null => Ok(Expr::Null),
            TokenKind::Undefined => Ok(Expr::Undefined),
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
                // Try an arrow-function parameter list first: `(params)
                // [: ReturnType] => body`. On any mismatch, roll back and
                // fall through to the existing grouped-expression parse —
                // `(` already-consumed position is `after_lparen`.
                let after_lparen = self.pos;
                if let Some(arrow) = self.try_parse_arrow_after_lparen()? {
                    return Ok(arrow);
                }
                self.pos = after_lparen;
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "`)` after grouped expression")?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                // Array literal: `[expr, ...spread, expr]` or `[]`.
                // The `[` was already consumed by `self.advance()` above.
                let mut elements = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    loop {
                        if self.matches(&TokenKind::DotDotDot) {
                            elements.push(ArrayElement::Spread(self.parse_expr()?));
                        } else {
                            elements.push(ArrayElement::Item(self.parse_expr()?));
                        }
                        if !self.matches(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RBracket, "`]` closing array literal")?;
                Ok(Expr::ArrayLiteral { elements })
            }
            TokenKind::LBrace => {
                // Object literal: `{ key: expr, ...spread }`.
                // Must be in a context with a declared type (let/const); the
                // codegen infers the type from that context.
                let mut fields = Vec::new();
                if !self.check(&TokenKind::RBrace) {
                    loop {
                        if self.matches(&TokenKind::DotDotDot) {
                            fields.push(ObjectField::Spread(self.parse_expr()?));
                        } else {
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
                            fields.push(ObjectField::KV(key, value));
                        }
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