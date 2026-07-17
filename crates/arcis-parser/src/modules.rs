//! Module parsing: `import` and `export`.
//!
//! Both statements follow TypeScript / ES-module syntax:
//! - `import [def,] { a, b as c } from "mod";`
//! - `export default ...` / `export { a, b as c }` /
//!   `export function/const/let ...`

use arcis_ast::{ExportDefault, ExportItem, ImportNamed, Stmt};
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

/// `import [def,] { a, b as c } from "mod";`
/// At least one of `def` or the named list must be present.
pub(crate) fn parse_import(p: &mut Parser) -> Result<Stmt, ParseError> {
    p.advance(); // import

    let mut default: Option<String> = None;
    let mut named: Vec<ImportNamed> = Vec::new();

    // Default binding: identifier followed by `,` or `from`.
    if let TokenKind::Ident(_) = p.peek_kind() {
        let after = p.peek_at(1).map(|t| &t.kind);
        if matches!(after, Some(TokenKind::From) | Some(TokenKind::Comma)) {
            let tok = p.advance();
            if let TokenKind::Ident(s) = tok.kind {
                default = Some(s);
            }
        }
    }

    // Optional `,` separator between default and the named list.
    if default.is_some() {
        p.matches(&TokenKind::Comma);
    }

    // Named list: `{ a, b as c }`
    if p.matches(&TokenKind::LBrace) {
        if !p.check(&TokenKind::RBrace) {
            loop {
                let name_tok = p.expect(&TokenKind::Ident(String::new()), "name in import")?;
                let name = match name_tok.kind {
                    TokenKind::Ident(s) => s,
                    _ => unreachable!(),
                };
                let alias = if p.matches(&TokenKind::As) {
                    let a = p.expect(&TokenKind::Ident(String::new()), "name after `as`")?;
                    match a.kind {
                        TokenKind::Ident(s) => Some(s),
                        _ => unreachable!(),
                    }
                } else {
                    None
                };
                named.push(ImportNamed { name, alias });
                if !p.matches(&TokenKind::Comma) {
                    break;
                }
            }
        }
        p.expect(&TokenKind::RBrace, "`}` closing import list")?;
    }

    if default.is_none() && named.is_empty() {
        let t = p.peek();
        return Err(ParseError {
            line: t.line,
            col: t.col,
            msg: "an `import` must have at least one binding (default or named)".to_string(),
        });
    }

    p.expect(&TokenKind::From, "`from` after import bindings")?;
    let module = p.expect_string_literal("module path")?;
    p.expect(&TokenKind::Semi, "`;` after `import`")?;

    Ok(Stmt::Import { default, named, module })
}

/// `export default ... | export { ... } | export function/const/let ...`
pub(crate) fn parse_export(p: &mut Parser) -> Result<Stmt, ParseError> {
    p.advance(); // export

    // export default ...
    if p.matches(&TokenKind::Default) {
        if p.check(&TokenKind::Function) {
            p.advance(); // function
            let f = p.parse_function_rest(false)?;
            return Ok(Stmt::ExportDefault(ExportDefault::Function(f)));
        }
        let expr = p.parse_expr()?;
        p.expect(&TokenKind::Semi, "`;` after `export default`")?;
        return Ok(Stmt::ExportDefault(ExportDefault::Expr(expr)));
    }

    // export { a, b as c };
    if p.matches(&TokenKind::LBrace) {
        let mut items = Vec::new();
        if !p.check(&TokenKind::RBrace) {
            loop {
                let name_tok = p.expect(&TokenKind::Ident(String::new()), "name in export")?;
                let name = match name_tok.kind {
                    TokenKind::Ident(s) => s,
                    _ => unreachable!(),
                };
                let alias = if p.matches(&TokenKind::As) {
                    let a = p.expect(&TokenKind::Ident(String::new()), "name after `as`")?;
                    match a.kind {
                        TokenKind::Ident(s) => Some(s),
                        _ => unreachable!(),
                    }
                } else {
                    None
                };
                items.push(ExportItem { name, alias });
                if !p.matches(&TokenKind::Comma) {
                    break;
                }
            }
        }
        p.expect(&TokenKind::RBrace, "`}` closing export list")?;
        p.expect(&TokenKind::Semi, "`;` after export")?;
        return Ok(Stmt::ExportSpec(items));
    }

    // export function / const / let (inline declaration)
    match p.peek_kind() {
        TokenKind::Function => {
            let stmt = p.parse_function()?;
            Ok(Stmt::ExportDecl(Box::new(stmt)))
        }
        TokenKind::Const => {
            let stmt = p.parse_let(true)?;
            Ok(Stmt::ExportDecl(Box::new(stmt)))
        }
        TokenKind::Let => {
            let stmt = p.parse_let(false)?;
            Ok(Stmt::ExportDecl(Box::new(stmt)))
        }
        other => {
            let t = p.peek();
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