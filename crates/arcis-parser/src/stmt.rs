//! Statement parsing.
//!
//! Dispatches on the first token of a statement to one of the per-kind
//! parsers. Each per-kind parser consumes the keyword, then the rest of
//! its production. The `for` and function-body parsers recurse back into
//! [`parse_stmt`](Parser::parse_stmt).

use arcis_ast::{Expr, Function, Param, Stmt, SwitchCase, Type};
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

impl Parser {
    /// Parse a single top-level or nested statement.
    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek_kind() {
            TokenKind::Let => self.parse_let(false),
            TokenKind::Const => self.parse_let(true),
            TokenKind::Function => self.parse_function(),
            TokenKind::Type => self.parse_type_alias(),
            TokenKind::Interface => self.parse_interface(),
            TokenKind::Enum => self.parse_enum(),
            TokenKind::Import => super::modules::parse_import(self),
            TokenKind::From => super::modules::parse_from_import(self),
            TokenKind::Export => super::modules::parse_export(self),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Switch => self.parse_switch(),
            TokenKind::Try => self.parse_try(),
            TokenKind::Throw => self.parse_throw(),
            TokenKind::Break => {
                self.advance();
                self.expect(&TokenKind::Semi, "`;` after `break`")?;
                Ok(Stmt::Break)
            }
            TokenKind::Continue => {
                self.advance();
                self.expect(&TokenKind::Semi, "`;` after `continue`")?;
                Ok(Stmt::Continue)
            }
            TokenKind::Ident(_) => {
                // Detect indexed assignment: `arr[expr] = expr;`
                // Or member assignment: `obj.field = expr;`
                // If the current token is an identifier and the next is `[` or
                // `.`, parse the full expression and check whether an `=` follows.
                let next_is_index_or_member = matches!(
                    self.peek_at(1).map(|t| &t.kind),
                    Some(TokenKind::LBracket) | Some(TokenKind::Dot)
                );
                if next_is_index_or_member {
                    let expr = self.parse_expr()?;
                    if self.check(&TokenKind::Eq) {
                        match expr {
                            Expr::Index { object, index } => {
                                let name = if let Expr::Ident(n) = *object {
                                    n
                                } else {
                                    return Err(ParseError {
                                        line: self.peek().line,
                                        col: self.peek().col,
                                        msg: "the LHS of an indexed assignment must be an identifier".to_string(),
                                    });
                                };
                                self.advance(); // =
                                let value = self.parse_expr()?;
                                self.expect(&TokenKind::Semi, "`;` after indexed assignment")?;
                                return Ok(Stmt::AssignIndex {
                                    object: name,
                                    index: *index,
                                    value,
                                });
                            }
                            Expr::Member { object, property } => {
                                self.advance(); // =
                                let value = self.parse_expr()?;
                                self.expect(&TokenKind::Semi, "`;` after member assignment")?;
                                return Ok(Stmt::AssignMember {
                                    object,
                                    property,
                                    value,
                                });
                            }
                            _ => {
                                return Err(ParseError {
                                    line: self.peek().line,
                                    col: self.peek().col,
                                    msg: "invalid assignment LHS".to_string(),
                                });
                            }
                        }
                    }
                    self.expect(&TokenKind::Semi, "after expression")?;
                    return Ok(Stmt::Expr(expr));
                }
                // Disambiguate: if the next token is `=` (not `==`), it's an
                // assignment; otherwise it's an expression.
                if matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::Eq)) {
                    self.parse_assign()
                } else {
                    let expr = self.parse_expr()?;
                    self.expect(&TokenKind::Semi, "after expression")?;
                    Ok(Stmt::Expr(expr))
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::Semi, "after expression")?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    /// Plain assignment: `IDENT = expr;`
    pub(crate) fn parse_assign(&mut self) -> Result<Stmt, ParseError> {
        let name_tok = self.advance(); // Ident
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!("parse_assign called without an Ident"),
        };
        self.expect(&TokenKind::Eq, "`=` after name")?;
        let value = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "`;` after value")?;
        Ok(Stmt::Assign { name, value })
    }

    /// Like [`parse_assign`](Self::parse_assign) but does not consume the
    /// trailing `;`. Used for the `update` slot of a C-style `for`.
    pub(crate) fn parse_assign_no_semi(&mut self) -> Result<Stmt, ParseError> {
        let name_tok = self.advance();
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        // Detect indexed assignment: `name[expr] = expr`
        if self.check(&TokenKind::LBracket) {
            self.advance(); // [
            let index = self.parse_expr()?;
            self.expect(&TokenKind::RBracket, "`]` in indexed assignment")?;
            self.expect(&TokenKind::Eq, "`=` in indexed assignment")?;
            let value = self.parse_expr()?;
            return Ok(Stmt::AssignIndex { object: name, index, value });
        }
        self.expect(&TokenKind::Eq, "`=` after name")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Assign { name, value })
    }

    /// `let` / `const` declaration. `is_const` distinguishes the two.
    pub(crate) fn parse_let(&mut self, is_const: bool) -> Result<Stmt, ParseError> {
        self.advance(); // let / const
        let name_tok = self.expect(&TokenKind::Ident(String::new()), "variable name")?;
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        let line = name_tok.line;
        let col = name_tok.col;

        let ty = if self.matches(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&TokenKind::Eq, "assignment `=`")?;
        let value = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "after value")?;

        if is_const {
            Ok(Stmt::Const { name, ty, value, line, col })
        } else {
            Ok(Stmt::Let { name, ty, value, line, col })
        }
    }

    /// `function NAME (params): RET { body }`
    pub(crate) fn parse_function(&mut self) -> Result<Stmt, ParseError> {
        let fn_tok = self.advance(); // function
        let fn_line = fn_tok.line;
        let fn_col = fn_tok.col;
        let f = self.parse_function_rest(true, fn_line, fn_col)?;
        Ok(Stmt::Function(f))
    }

    /// Parse `[name] (params) : return { body }`. Called immediately after
    /// consuming `function`. If `require_name` is `false` (e.g.
    /// `export default function`), an anonymous function (empty name) is
    /// permitted.
    pub(crate) fn parse_function_rest(
        &mut self,
        require_name: bool,
        fn_keyword_line: usize,
        fn_keyword_col: usize,
    ) -> Result<Function, ParseError> {
        let (name, name_line, name_col) = if let TokenKind::Ident(_) = self.peek_kind() {
            let tok = self.advance();
            let line = tok.line;
            let col = tok.col;
            match tok.kind {
                TokenKind::Ident(s) => (s, line, col),
                _ => unreachable!(),
            }
        } else if require_name {
            let t = self.peek();
            return Err(ParseError {
                line: t.line,
                col: t.col,
                msg: "expected function name".to_string(),
            });
        } else {
            (String::new(), fn_keyword_line, fn_keyword_col)
        };

        self.expect(&TokenKind::LParen, "`(` after function name")?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let pname_tok = self.expect(&TokenKind::Ident(String::new()), "parameter name")?;
                let pname = match &pname_tok.kind {
                    TokenKind::Ident(s) => s.clone(),
                    _ => unreachable!(),
                };
                let pline = pname_tok.line;
                let pcol = pname_tok.col;
                self.expect(&TokenKind::Colon, "`:` after parameter name")?;
                let pty = self.parse_type()?;
                params.push(Param { name: pname, ty: pty, line: pline, col: pcol });
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "`)` after parameters")?;

        let return_type = if self.matches(&TokenKind::Colon) {
            self.parse_type()?
        } else {
            Type::void()
        };

        self.expect(&TokenKind::LBrace, "`{` opening function body")?;
        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}` closing function body")?;

        Ok(Function {
            name,
            params,
            return_type,
            body,
            line: name_line,
            col: name_col,
        })
    }

    /// `type Name = <type>;`
    pub(crate) fn parse_type_alias(&mut self) -> Result<Stmt, ParseError> {
        let kw_tok = self.advance(); // type
        let name_tok = self.expect(&TokenKind::Ident(String::new()), "type alias name")?;
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        self.expect(&TokenKind::Eq, "`=` after type alias name")?;
        let ty = self.parse_type()?;
        self.expect(&TokenKind::Semi, "`;` after type alias")?;
        Ok(Stmt::TypeAlias {
            name,
            ty,
            line: kw_tok.line,
            col: kw_tok.col,
        })
    }

    /// `interface Name [extends Base, ...] { field: type, field2?: type }`
    pub(crate) fn parse_interface(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // interface
        let name_tok = self.expect(&TokenKind::Ident(String::new()), "interface name")?;
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        let mut extends = Vec::new();
        if self.matches(&TokenKind::Extends) {
            loop {
                let base = self.expect_ident("base interface name")?;
                extends.push(base);
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::LBrace, "`{` opening interface body")?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            let key = self.expect_ident("field name in interface")?;
            let optional = self.matches(&TokenKind::Question);
            self.expect(&TokenKind::Colon, "`:` after field name")?;
            let field_ty = self.parse_type()?;
            // `field?: T` carries the same meaning as `field: T?` — fold the
            // `?` marker into the type itself so the null-safety checker
            // (which only looks at `Type::Optional`) sees it too.
            let stored_ty = if optional { arcis_ast::Type::optional(field_ty) } else { field_ty };
            fields.push((key, Box::new(stored_ty), optional));
            // `,` or `;` both separate fields; trailing separator is optional.
            let _ = self.matches(&TokenKind::Comma) || self.matches(&TokenKind::Semi);
        }
        self.expect(&TokenKind::RBrace, "`}` closing interface body")?;
        Ok(Stmt::Interface {
            name,
            extends,
            fields,
            line: name_tok.line,
            col: name_tok.col,
        })
    }

    /// `enum Name { A, B = 5, C }`
    pub(crate) fn parse_enum(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // enum
        let name_tok = self.expect(&TokenKind::Ident(String::new()), "enum name")?;
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        self.expect(&TokenKind::LBrace, "`{` opening enum body")?;
        let mut variants = Vec::new();
        if !self.check(&TokenKind::RBrace) {
            loop {
                let variant_name = self.expect_ident("enum variant name")?;
                let value = if self.matches(&TokenKind::Eq) {
                    let tok = self.advance();
                    match tok.kind {
                        TokenKind::Number(n) => Some(n as i64),
                        other => {
                            return Err(ParseError {
                                line: tok.line,
                                col: tok.col,
                                msg: format!("expected a numeric enum value, found {}", other),
                            });
                        }
                    }
                } else {
                    None
                };
                variants.push((variant_name, value));
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
                // Allow a trailing comma before `}`.
                if self.check(&TokenKind::RBrace) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RBrace, "`}` closing enum body")?;
        Ok(Stmt::Enum {
            name,
            variants,
            line: name_tok.line,
            col: name_tok.col,
        })
    }

    /// `return [expr];`
    pub(crate) fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // return
        if self.check(&TokenKind::Semi) {
            self.advance();
            return Ok(Stmt::Return(None));
        }
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "`;` after `return`")?;
        Ok(Stmt::Return(Some(expr)))
    }

    /// `if (cond) { then } [else if (cond2) { ... }]* [else { els }]`
    ///
    /// Supports `else if` chains by recursively parsing the next `if`
    /// statement when we see `else if`. The chain is flattened into a
    /// single `Stmt::If` with the recursive `else` branch as another
    /// `Stmt::If`.
    pub(crate) fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // if
        self.expect(&TokenKind::LParen, "`(` after `if`")?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "`)` after if condition")?;
        self.expect(&TokenKind::LBrace, "`{` opening then-block")?;
        let mut then_branch = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            then_branch.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}` closing then-block")?;

        let else_branch = if self.matches(&TokenKind::Else) {
            // `else if (cond) { ... }` — recurse into another `if` so the
            // chain is parsed naturally. The desugaring of `else if` to
            // nested `Stmt::If` is what the codegen already expects.
            if self.check(&TokenKind::If) {
                let inner_if = self.parse_if()?;
                Some(vec![inner_if])
            } else {
                self.expect(&TokenKind::LBrace, "`{` opening else-block")?;
                let mut stmts = Vec::new();
                while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
                    stmts.push(self.parse_stmt()?);
                }
                self.expect(&TokenKind::RBrace, "`}` closing else-block")?;
                Some(stmts)
            }
        } else {
            None
        };

        Ok(Stmt::If { condition, then_branch, else_branch })
    }

    /// `while (cond) { body }`
    pub(crate) fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // while
        self.expect(&TokenKind::LParen, "`(` after `while`")?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "`)` after while condition")?;
        self.expect(&TokenKind::LBrace, "`{` opening while body")?;
        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}` closing while body")?;
        Ok(Stmt::While { condition, body })
    }

    /// `for (init; cond; update) { body }` or `for (let x of arr) { body }`.
    pub(crate) fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // for
        self.expect(&TokenKind::LParen, "`(` after `for`")?;

        // Detect for-of: `for (let IDENT of EXPR)`.
        // The C-style init pattern `let IDENT = EXPR` has `=` at peek_at(2),
        // while for-of has `of` at peek_at(2).
        let is_for_of = matches!(self.peek_kind(), TokenKind::Let)
            && matches!(
                self.peek_at(1).map(|t| std::mem::discriminant(&t.kind)),
                Some(d) if d == std::mem::discriminant(&TokenKind::Ident(String::new()))
            )
            && matches!(self.peek_at(2).map(|t| &t.kind), Some(TokenKind::Of));

        if is_for_of {
            self.advance(); // let
            let name_tok =
                self.expect(&TokenKind::Ident(String::new()), "loop variable in `for-of`")?;
            let name = match &name_tok.kind {
                TokenKind::Ident(s) => s.clone(),
                _ => unreachable!(),
            };
            // Optional type: `let x: number of arr`
            let ty = if self.matches(&TokenKind::Colon) {
                Some(self.parse_type()?)
            } else {
                None
            };
            self.expect(&TokenKind::Of, "`of` after loop variable")?;
            let iterable = self.parse_expr()?;
            self.expect(&TokenKind::RParen, "`)` after `for-of` iterable")?;
            self.expect(&TokenKind::LBrace, "`{` opening `for-of` body")?;
            let mut body = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
                body.push(self.parse_stmt()?);
            }
            self.expect(&TokenKind::RBrace, "`}` closing `for-of` body")?;
            return Ok(Stmt::ForOf { name, ty, iterable: Box::new(iterable), body });
        }

        // init: let | const | expr | empty (each terminated with `;`)
        let init = if self.check(&TokenKind::Semi) {
            self.advance();
            None
        } else if matches!(self.peek_kind(), TokenKind::Let | TokenKind::Const) {
            let stmt = self.parse_let(matches!(self.peek_kind(), TokenKind::Const))?;
            // parse_let already consumed the `;`
            Some(Box::new(stmt))
        } else {
            // expression
            let expr = self.parse_expr()?;
            self.expect(&TokenKind::Semi, "`;` after `for` init")?;
            Some(Box::new(Stmt::Expr(expr)))
        };

        // condition: optional
        let condition = if self.check(&TokenKind::Semi) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(&TokenKind::Semi, "`;` after `for` condition")?;

        // update: optional
        let update = if self.check(&TokenKind::RParen) {
            None
        } else {
            // can be an assignment or expression statement.
            // Detect assignment the same way as parse_stmt but without `;`.
            let stmt = if let TokenKind::Ident(_) = self.peek_kind() {
                if matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::Eq)) {
                    self.parse_assign_no_semi()?
                } else {
                    let expr = self.parse_expr()?;
                    Stmt::Expr(expr)
                }
            } else {
                let expr = self.parse_expr()?;
                Stmt::Expr(expr)
            };
            Some(Box::new(stmt))
        };

        self.expect(&TokenKind::RParen, "`)` after `for` update")?;
        self.expect(&TokenKind::LBrace, "`{` opening `for` body")?;
        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}` closing `for` body")?;

        Ok(Stmt::For { init, condition, update, body })
    }

    /// `switch (discriminant) { case v1: stmt* case v2: stmt* default: stmt* }`
    ///
    /// Consecutive `case` labels with no statements between them share one
    /// body (`case v1: case v2: stmt*` -> one [`SwitchCase`] with
    /// `values: [v1, v2]`), matching Rust's `v1 | v2 => { }` or-pattern.
    /// Non-fallthrough otherwise: each case's body is its own block, not a
    /// C-style fallthrough chain (see `docs/language-reference.md`).
    pub(crate) fn parse_switch(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // switch
        self.expect(&TokenKind::LParen, "`(` after `switch`")?;
        let discriminant = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "`)` after switch discriminant")?;
        self.expect(&TokenKind::LBrace, "`{` opening switch body")?;

        let mut cases = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            let mut values = Vec::new();
            let mut is_default = false;
            loop {
                if self.matches(&TokenKind::Case) {
                    values.push(self.parse_expr()?);
                    self.expect(&TokenKind::Colon, "`:` after `case` value")?;
                } else if self.matches(&TokenKind::Default) {
                    is_default = true;
                    self.expect(&TokenKind::Colon, "`:` after `default`")?;
                } else {
                    break;
                }
                if !matches!(self.peek_kind(), TokenKind::Case | TokenKind::Default) {
                    break;
                }
            }
            if values.is_empty() && !is_default {
                let t = self.peek();
                return Err(ParseError {
                    line: t.line,
                    col: t.col,
                    msg: "expected `case` or `default` in switch body".to_string(),
                });
            }
            let mut body = Vec::new();
            while !matches!(self.peek_kind(), TokenKind::Case | TokenKind::Default | TokenKind::RBrace)
                && !self.check(&TokenKind::Eof)
            {
                body.push(self.parse_stmt()?);
            }
            cases.push(SwitchCase { values, body, is_default });
        }
        self.expect(&TokenKind::RBrace, "`}` closing switch body")?;
        Ok(Stmt::Switch { discriminant, cases })
    }

    /// `try { body } catch (e) { catch_body }`.
    pub(crate) fn parse_try(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // try
        self.expect(&TokenKind::LBrace, "`{` opening `try` body")?;
        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}` closing `try` body")?;

        self.expect(&TokenKind::Catch, "`catch` after `try` body")?;
        let catch_name = if self.matches(&TokenKind::LParen) {
            let name = self.expect_ident("caught error name")?;
            self.expect(&TokenKind::RParen, "`)` after caught error name")?;
            Some(name)
        } else {
            None
        };
        self.expect(&TokenKind::LBrace, "`{` opening `catch` body")?;
        let mut catch_body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            catch_body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}` closing `catch` body")?;

        Ok(Stmt::Try { body, catch_name, catch_body })
    }

    /// `throw expr;`
    pub(crate) fn parse_throw(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // throw
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "`;` after `throw`")?;
        Ok(Stmt::Throw(expr))
    }
}