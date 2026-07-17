//! Arcis parser: token stream → AST.
//!
//! Recursive-descent parser with **precedence climbing** for expressions.
//!
//! Grammar (simplified):
//! ```text
//!   program     = stmt*
//!   stmt        = let_stmt | const_stmt | fn_stmt | if_stmt | while_stmt
//!               | for_stmt | return_stmt | break_stmt | continue_stmt
//!               | assign_stmt | expr_stmt
//!   let_stmt    = "let"  IDENT (":" type)? "=" expr ";"
//!   const_stmt  = "const" IDENT (":" type)? "=" expr ";"
//!   fn_stmt     = "function" IDENT "(" params? ")" (":" type)? "{" stmt* "}"
//!   if_stmt     = "if" "(" expr ")" "{" stmt* "}" ("else" "{" stmt* "}")?
//!   while_stmt  = "while" "(" expr ")" "{" stmt* "}"
//!   for_stmt    = "for" "(" (let_stmt | expr_stmt | ";") expr? ";" expr? ")" "{" stmt* "}"
//!   return_stmt = "return" expr? ";"
//!   break_stmt  = "break" ";"
//!   continue_stmt = "continue" ";"
//!   expr_stmt   = expr ";"
//!
//!   expr        = or
//!   or          = and ( "||" and )*
//!   and         = equality ( "&&" equality )*
//!   equality    = comparison ( ("==" | "!=") comparison )*
//!   comparison  = additive ( ("<" | ">" | "<=" | ">=") additive )*
//!   additive    = multiplicative ( ("+" | "-") multiplicative )*
//!   multiplicative = unary ( ("*" | "/" | "%") unary )*
//!   unary       = ("!" | "-") unary | postfix
//!   postfix     = atom ( "." IDENT | "[" expr "]" )*
//!   atom        = NUMBER | STRING | "true" | "false" | IDENT ("(" args ")")? | "(" expr ")"
//! ```
//!
//! In phase 2 this single file will be split into `state.rs` (the `Parser`
//! struct + look-ahead helpers), `error.rs`, `stmt.rs`, `expr.rs`, `types.rs`,
//! and `modules.rs` for `import`/`export`.

use arcis_ast::{
    BinOp, ExportDefault, ExportItem, Expr, Function, ImportNamed, Param, Program, Stmt, Type,
    UnaryOp,
};
use arcis_lexer::{Token, TokenKind};

/// Generate a deterministic identifier for an inline object type, based on the
/// hash of its field shape. Same shape → same name → same struct.
fn object_type_name(fields: &[(String, Box<Type>)]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (k, t) in fields {
        k.hash(&mut h);
        t.name.hash(&mut h);
        for (fk, ft) in &t.fields {
            fk.hash(&mut h);
            ft.name.hash(&mut h);
        }
    }
    format!("__Obj{:x}", h.finish() & 0xFFFFFF)
}

/// Parse a complete token stream into a [`Program`].
pub fn parse(tokens: Vec<Token>) -> Result<Program, ParseError> {
    let mut p = Parser { tokens, pos: 0 };
    let mut stmts = Vec::new();
    while !p.check(&TokenKind::Eof) {
        stmts.push(p.parse_stmt()?);
    }
    Ok(Program { stmts })
}

/// Parser error returned by [`parse`].
#[derive(Debug)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parse error at {}:{}: {}", self.line, self.col, self.msg)
    }
}

impl std::error::Error for ParseError {}

/// Mutable parser state: the full token stream and a cursor position.
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    /// Look-ahead: returns the token at `pos + offset` without consuming.
    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind)
    }

    fn matches(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, context: &str) -> Result<Token, ParseError> {
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

    // ── Statements ────────────────────────────────────────────────────────

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek_kind() {
            TokenKind::Let => self.parse_let(false),
            TokenKind::Const => self.parse_let(true),
            TokenKind::Function => self.parse_function(),
            TokenKind::Import => self.parse_import(),
            TokenKind::Export => self.parse_export(),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
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

    fn parse_assign(&mut self) -> Result<Stmt, ParseError> {
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

    fn parse_let(&mut self, is_const: bool) -> Result<Stmt, ParseError> {
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

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        // Inline object type: { name: type, ... }
        if self.check(&TokenKind::LBrace) {
            self.advance(); // {
            let mut fields = Vec::new();
            // Permit `{}` (empty object type)
            if !self.check(&TokenKind::RBrace) {
                loop {
                    let key_tok = self.expect(
                        &TokenKind::Ident(String::new()),
                        "field name in object type",
                    )?;
                    let key = match &key_tok.kind {
                        TokenKind::Ident(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    self.expect(&TokenKind::Colon, "`:` after field name")?;
                    let field_ty = self.parse_type()?;
                    fields.push((key, Box::new(field_ty)));
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RBrace, "`}` closing object type")?;
            // Generate a deterministic, unique name based on the shape.
            let name = object_type_name(&fields);
            let mut is_array = false;
            // Optional `[]` suffix (array of objects).
            if self.check(&TokenKind::LBracket) {
                self.advance();
                self.expect(&TokenKind::RBracket, "`]` after `[`")?;
                is_array = true;
            }
            return Ok(Type { name, fields, is_array });
        }

        let t = self.advance();
        let name = match &t.kind {
            TokenKind::TypeString => "string".to_string(),
            TokenKind::TypeNumber => "number".to_string(),
            TokenKind::TypeBoolean => "boolean".to_string(),
            TokenKind::TypeVoid => "void".to_string(),
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(ParseError {
                    line: t.line,
                    col: t.col,
                    msg: format!("unknown type {}", other),
                });
            }
        };
        let mut is_array = false;
        // Optional `[]` suffix (one level in v1).
        if self.check(&TokenKind::LBracket) {
            self.advance();
            self.expect(&TokenKind::RBracket, "`]` after `[`")?;
            is_array = true;
        }
        Ok(Type { name, fields: Vec::new(), is_array })
    }

    fn parse_function(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // function
        let f = self.parse_function_rest(true)?;
        Ok(Stmt::Function(f))
    }

    /// Parse `[name] (params) : return { body }`.
    /// Called immediately after consuming `function`.
    /// If `require_name` is `false` (e.g. `export default function`) an
    /// anonymous function (empty name) is permitted.
    fn parse_function_rest(&mut self, require_name: bool) -> Result<Function, ParseError> {
        let name = if let TokenKind::Ident(_) = self.peek_kind() {
            let tok = self.advance();
            match tok.kind {
                TokenKind::Ident(s) => s,
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
            String::new()
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
        })
    }

    // ── Imports / Exports (ES modules / TS) ───────────────────────────────

    /// Consume a string-literal token and return its contents.
    fn expect_string_literal(&mut self, context: &str) -> Result<String, ParseError> {
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

    /// `import [def,] { a, b as c } from "mod";`
    /// At least one of `def` or the named list must be present.
    fn parse_import(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // import

        let mut default: Option<String> = None;
        let mut named: Vec<ImportNamed> = Vec::new();

        // Default binding: identifier followed by `,` or `from`.
        if let TokenKind::Ident(_) = self.peek_kind() {
            let after = self.peek_at(1).map(|t| &t.kind);
            if matches!(after, Some(TokenKind::From) | Some(TokenKind::Comma)) {
                let tok = self.advance();
                if let TokenKind::Ident(s) = tok.kind {
                    default = Some(s);
                }
            }
        }

        // Optional `,` separator between default and the named list.
        if default.is_some() {
            self.matches(&TokenKind::Comma);
        }

        // Named list: `{ a, b as c }`
        if self.matches(&TokenKind::LBrace) {
            if !self.check(&TokenKind::RBrace) {
                loop {
                    let name_tok = self.expect(&TokenKind::Ident(String::new()), "name in import")?;
                    let name = match name_tok.kind {
                        TokenKind::Ident(s) => s,
                        _ => unreachable!(),
                    };
                    let alias = if self.matches(&TokenKind::As) {
                        let a = self.expect(&TokenKind::Ident(String::new()), "name after `as`")?;
                        match a.kind {
                            TokenKind::Ident(s) => Some(s),
                            _ => unreachable!(),
                        }
                    } else {
                        None
                    };
                    named.push(ImportNamed { name, alias });
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RBrace, "`}` closing import list")?;
        }

        if default.is_none() && named.is_empty() {
            let t = self.peek();
            return Err(ParseError {
                line: t.line,
                col: t.col,
                msg: "an `import` must have at least one binding (default or named)"
                    .to_string(),
            });
        }

        self.expect(&TokenKind::From, "`from` after import bindings")?;
        let module = self.expect_string_literal("module path")?;
        self.expect(&TokenKind::Semi, "`;` after `import`")?;

        Ok(Stmt::Import { default, named, module })
    }

    /// `export default ... | export { ... } | export function/const/let ...`
    fn parse_export(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // export

        // export default ...
        if self.matches(&TokenKind::Default) {
            if self.check(&TokenKind::Function) {
                self.advance(); // function
                let f = self.parse_function_rest(false)?;
                return Ok(Stmt::ExportDefault(ExportDefault::Function(f)));
            }
            let expr = self.parse_expr()?;
            self.expect(&TokenKind::Semi, "`;` after `export default`")?;
            return Ok(Stmt::ExportDefault(ExportDefault::Expr(expr)));
        }

        // export { a, b as c };
        if self.matches(&TokenKind::LBrace) {
            let mut items = Vec::new();
            if !self.check(&TokenKind::RBrace) {
                loop {
                    let name_tok = self.expect(&TokenKind::Ident(String::new()), "name in export")?;
                    let name = match name_tok.kind {
                        TokenKind::Ident(s) => s,
                        _ => unreachable!(),
                    };
                    let alias = if self.matches(&TokenKind::As) {
                        let a = self.expect(&TokenKind::Ident(String::new()), "name after `as`")?;
                        match a.kind {
                            TokenKind::Ident(s) => Some(s),
                            _ => unreachable!(),
                        }
                    } else {
                        None
                    };
                    items.push(ExportItem { name, alias });
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RBrace, "`}` closing export list")?;
            self.expect(&TokenKind::Semi, "`;` after export")?;
            return Ok(Stmt::ExportSpec(items));
        }

        // export function / const / let (inline declaration)
        match self.peek_kind() {
            TokenKind::Function => {
                let stmt = self.parse_function()?;
                Ok(Stmt::ExportDecl(Box::new(stmt)))
            }
            TokenKind::Const => {
                let stmt = self.parse_let(true)?;
                Ok(Stmt::ExportDecl(Box::new(stmt)))
            }
            TokenKind::Let => {
                let stmt = self.parse_let(false)?;
                Ok(Stmt::ExportDecl(Box::new(stmt)))
            }
            other => {
                let t = self.peek();
                Err(ParseError {
                    line: t.line,
                    col: t.col,
                    msg: format!(
                        "invalid export: expected `default`, `{{`, `function`, `const`, or `let`, found {}",
                        other
                    ),
                })
            }
        }
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // return
        if self.check(&TokenKind::Semi) {
            self.advance();
            return Ok(Stmt::Return(None));
        }
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "`;` after `return`")?;
        Ok(Stmt::Return(Some(expr)))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
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
            self.expect(&TokenKind::LBrace, "`{` opening else-block")?;
            let mut stmts = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
                stmts.push(self.parse_stmt()?);
            }
            self.expect(&TokenKind::RBrace, "`}` closing else-block")?;
            Some(stmts)
        } else {
            None
        };

        Ok(Stmt::If { condition, then_branch, else_branch })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
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

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
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
            let name_tok = self.expect(&TokenKind::Ident(String::new()), "loop variable in `for-of`")?;
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
                    self.parse_assign_no_semi()
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

    /// Like [`parse_assign`] but does not consume the trailing `;`. Used for
    /// the `update` slot of a C-style `for`.
    fn parse_assign_no_semi(&mut self) -> Stmt {
        let name_tok = self.advance();
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        // Detect indexed assignment: `name[expr] = expr`
        if self.check(&TokenKind::LBracket) {
            self.advance(); // [
            let index = self.parse_expr().unwrap();
            self.expect(&TokenKind::RBracket, "`]` in indexed assignment").unwrap();
            self.expect(&TokenKind::Eq, "`=` in indexed assignment").unwrap();
            let value = self.parse_expr().unwrap();
            return Stmt::AssignIndex { object: name, index, value };
        }
        self.expect(&TokenKind::Eq, "`=` after name").unwrap();
        let value = self.parse_expr().unwrap();
        Stmt::Assign { name, value }
    }

    // ── Expressions (precedence climbing) ─────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Binary { op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::And) {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::Binary { op: BinOp::And, left: Box::new(left), right: Box::new(right) };
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
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
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
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
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
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
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
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Bang => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Not, operand: Box::new(operand) })
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Neg, operand: Box::new(operand) })
            }
            _ => self.parse_postfix(),
        }
    }

    /// Parse a postfix: from an atom, consume `.ident`, `[expr]`, and `(args)`
    /// chains to construct `Member`/`Index`/`Call` nodes.
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
                    expr = Expr::Member { object: Box::new(expr), property };
                }
                TokenKind::LBracket => {
                    self.advance(); // [
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket, "`]` after index expression")?;
                    expr = Expr::Index { object: Box::new(expr), index: Box::new(index) };
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
                    expr = Expr::Call { callee: Box::new(expr), args };
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
                // Static path: `crate::Type::method`. Only valid at the start of
                // an expression (not chained off another expression).
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
                    Ok(Expr::Call { callee: Box::new(Expr::Ident(name)), args })
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcis_lexer::lex;

    #[test]
    fn parses_simple_let_and_print() {
        let toks = lex("let x: number = 42; print(x);").expect("lex must succeed");
        let prog = parse(toks).expect("parse must succeed");
        assert_eq!(prog.stmts.len(), 2);
    }

    #[test]
    fn parses_module_import() {
        let toks = lex("import { add } from \"utils\";").expect("lex must succeed");
        let prog = parse(toks).expect("parse must succeed");
        assert!(matches!(prog.stmts[0], Stmt::Import { .. }));
    }
}