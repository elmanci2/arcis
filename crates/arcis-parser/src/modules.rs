//! Module parsing: `import` and `export` (Python-style).
//!
//! Imports:
//! - `import utils`                 → namespace import
//! - `import utils as u`            → namespace import with alias
//! - `import os.path`              → dotted module path
//! - `from utils import a`         → single named import
//! - `from utils import a, b as c` → multiple named imports
//! - `from utils import *`         → wildcard import
//!
//! Exports (unchanged from ES-module style):
//! - `export function f(){}` / `export const X = ...` / `export let Y = ...`
//! - `export { a, b as c };`
//! - `export default ...`

use arcis_ast::{ExportDefault, ExportItem, ImportNamed, Stmt};
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

/// Parse a dotted module path: `IDENT (. IDENT)*`.
/// Returns the list of path segments.
fn parse_module_path(p: &mut Parser) -> Result<Vec<String>, ParseError> {
    let first = p.expect_ident("module name")?;
    let mut path = vec![first];
    while p.matches(&TokenKind::Dot) {
        let next = p.expect_ident("module name after `.`")?;
        path.push(next);
    }
    Ok(path)
}

/// `import IDENT (. IDENT)* (as IDENT)? ;`
pub(crate) fn parse_import(p: &mut Parser) -> Result<Stmt, ParseError> {
    p.advance(); // import

    let module = parse_module_path(p)?;

    let alias = if p.matches(&TokenKind::As) {
        Some(p.expect_ident("alias after `as`")?)
    } else {
        None
    };

    p.expect(&TokenKind::Semi, "`;` after `import`")?;
    Ok(Stmt::Import { module, alias })
}

/// `from IDENT (. IDENT)* import names ;`
pub(crate) fn parse_from_import(p: &mut Parser) -> Result<Stmt, ParseError> {
    p.advance(); // from

    let module = parse_module_path(p)?;

    p.expect(
        &TokenKind::Import,
        "`import` after module path in `from ... import`",
    )?;

    // Wildcard: `from utils import * ;`
    if p.matches(&TokenKind::Star) {
        p.expect(&TokenKind::Semi, "`;` after `from ... import *`")?;
        return Ok(Stmt::FromImport {
            module,
            names: vec![],
            wildcard: true,
        });
    }

    // Named list: `IDENT (as IDENT)? (, IDENT (as IDENT)?)*`
    let mut names = Vec::new();

    let name = p.expect_ident("name after `import`")?;
    let alias = if p.matches(&TokenKind::As) {
        Some(p.expect_ident("alias after `as`")?)
    } else {
        None
    };
    names.push(ImportNamed { name, alias });

    while p.matches(&TokenKind::Comma) {
        let name = p.expect_ident("name after `,`")?;
        let alias = if p.matches(&TokenKind::As) {
            Some(p.expect_ident("alias after `as`")?)
        } else {
            None
        };
        names.push(ImportNamed { name, alias });
    }

    p.expect(&TokenKind::Semi, "`;` after `from ... import`")?;
    Ok(Stmt::FromImport {
        module,
        names,
        wildcard: false,
    })
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
                let name_tok =
                    p.expect(&TokenKind::Ident(String::new()), "name in export")?;
                let name = match name_tok.kind {
                    TokenKind::Ident(s) => s,
                    _ => unreachable!(),
                };
                let alias = if p.matches(&TokenKind::As) {
                    let a = p.expect(
                        &TokenKind::Ident(String::new()),
                        "name after `as`",
                    )?;
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
                    "invalid export: expected `default`, `{{`, `function`, \
                     `const`, or `let`, found {}",
                    other
                ),
            })
        }
    }
}
