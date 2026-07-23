//! Module parsing: `import` and `export` (ES/TS style + Python style).
//!
//! ES/TS-style imports (module specifier is a string literal or a bare path):
//! - `import { a, b as c } from "utils";`  → named imports
//! - `import def from "utils";`            → default import
//! - `import def, { a } from "utils";`     → default + named
//! - `import * as ns from "utils";`        → namespace import
//!
//! Python-style imports (kept for compatibility):
//! - `import utils`                 → namespace import
//! - `import utils as u`            → namespace import with alias
//! - `import os.path`              → dotted module path
//! - `from utils import a`         → single named import
//! - `from utils import a, b as c` → multiple named imports
//! - `from utils import *`         → wildcard import
//!
//! External Rust crates: `crate:<name>` as the first path segment, in either
//! style — `import { X } from "crate:serde";` / `from crate:serde import X;`.
//!
//! Exports (ES-module style):
//! - `export function f(){}` / `export const X = ...` / `export let Y = ...`
//! - `export type/interface/enum ...`
//! - `export { a, b as c };`
//! - `export default ...`

use arcis_ast::{ExportDefault, ExportItem, ImportNamed, Stmt};
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

/// Name used on [`ImportNamed::name`] to mean "the module's default export".
/// The linker resolves it against the target module's `export default`.
pub const DEFAULT_IMPORT: &str = "default";

/// Parse a dotted module path: `IDENT (. IDENT)*`.
/// The first segment may be a Rust-crate specifier `crate:NAME`.
/// Returns the list of path segments.
fn parse_module_path(p: &mut Parser) -> Result<Vec<String>, ParseError> {
    let mut first = p.expect_ident("module name")?;
    if first == "crate" && p.matches(&TokenKind::Colon) {
        let name = p.expect_ident("crate name after `crate:`")?;
        first = format!("crate:{}", name);
    }
    let mut path = vec![first];
    while p.matches(&TokenKind::Dot) {
        let next = p.expect_ident("module name after `.`")?;
        path.push(next);
    }
    Ok(path)
}

/// Convert a string module specifier (`"utils"`, `"./utils"`, `"dir/utils"`,
/// `"crate:serde"`) into path segments compatible with the linker resolver.
fn string_spec_to_path(spec: &str) -> Vec<String> {
    let trimmed = spec.strip_prefix("./").unwrap_or(spec);
    trimmed
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Module specifier after `from`: either a string literal (ES style) or a
/// bare dotted path (Python style).
fn parse_module_spec(p: &mut Parser) -> Result<Vec<String>, ParseError> {
    if p.check(&TokenKind::String(String::new())) {
        let tok = p.advance();
        let spec = match tok.kind {
            TokenKind::String(s) => s,
            _ => unreachable!(),
        };
        let segments = string_spec_to_path(&spec);
        if segments.is_empty() {
            return Err(ParseError {
                line: tok.line,
                col: tok.col,
                msg: "empty module specifier".to_string(),
            });
        }
        Ok(segments)
    } else {
        parse_module_path(p)
    }
}

/// `IDENT (as IDENT)? (, IDENT (as IDENT)?)*` — inside `{ ... }` for ES
/// imports or after `import` in a `from ... import` statement.
fn parse_named_list(p: &mut Parser) -> Result<Vec<ImportNamed>, ParseError> {
    let mut names = Vec::new();
    loop {
        // `default` is a keyword token but a valid import name
        // (`from utils import default as compute;`).
        let name = if p.matches(&TokenKind::Default) {
            DEFAULT_IMPORT.to_string()
        } else {
            p.expect_ident("imported name")?
        };
        let alias = if p.matches(&TokenKind::As) {
            Some(p.expect_ident("alias after `as`")?)
        } else {
            None
        };
        names.push(ImportNamed { name, alias });
        if !p.matches(&TokenKind::Comma) {
            break;
        }
    }
    Ok(names)
}

/// `import ...` — dispatches between ES/TS style and Python style:
///
/// - `import { a, b as c } from <spec>;` → named imports
/// - `import * as ns from <spec>;`       → namespace import with alias
/// - `import X from <spec>;`             → default import
/// - `import X, { a } from <spec>;`      → default + named
/// - `import utils (as u)?;`             → Python-style namespace import
pub(crate) fn parse_import(p: &mut Parser) -> Result<Stmt, ParseError> {
    p.advance(); // import

    // `import { a, b as c } from <spec>;`
    if p.matches(&TokenKind::LBrace) {
        let names = if p.check(&TokenKind::RBrace) {
            Vec::new()
        } else {
            parse_named_list(p)?
        };
        p.expect(&TokenKind::RBrace, "`}` closing import list")?;
        p.expect(&TokenKind::From, "`from` after import list")?;
        let module = parse_module_spec(p)?;
        p.expect(&TokenKind::Semi, "`;` after `import`")?;
        return Ok(Stmt::FromImport {
            module,
            names,
            wildcard: false,
        });
    }

    // `import * as ns from <spec>;`
    if p.matches(&TokenKind::Star) {
        p.expect(&TokenKind::As, "`as` after `import *`")?;
        let alias = p.expect_ident("namespace alias after `as`")?;
        p.expect(&TokenKind::From, "`from` after `import * as ns`")?;
        let module = parse_module_spec(p)?;
        p.expect(&TokenKind::Semi, "`;` after `import`")?;
        return Ok(Stmt::Import {
            module,
            alias: Some(alias),
        });
    }

    let module = parse_module_path(p)?;

    // `import X from <spec>;` / `import X, { a } from <spec>;` — the leading
    // identifier is a default-import binding, not a module path.
    if module.len() == 1
        && (p.check(&TokenKind::From) || p.check(&TokenKind::Comma))
    {
        let local = module.into_iter().next().unwrap();
        let mut names = vec![ImportNamed {
            name: DEFAULT_IMPORT.to_string(),
            alias: Some(local),
        }];
        if p.matches(&TokenKind::Comma) {
            p.expect(&TokenKind::LBrace, "`{` after `,` in import")?;
            if !p.check(&TokenKind::RBrace) {
                names.extend(parse_named_list(p)?);
            }
            p.expect(&TokenKind::RBrace, "`}` closing import list")?;
        }
        p.expect(&TokenKind::From, "`from` after import bindings")?;
        let module = parse_module_spec(p)?;
        p.expect(&TokenKind::Semi, "`;` after `import`")?;
        return Ok(Stmt::FromImport {
            module,
            names,
            wildcard: false,
        });
    }

    // Python-style namespace import: `import utils (as u)?;`
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
    let names = parse_named_list(p)?;

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
            let fn_tok = p.advance(); // function
            let fn_line = fn_tok.line;
            let fn_col = fn_tok.col;
            let f = p.parse_function_rest(false, fn_line, fn_col)?;
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
        TokenKind::Type => {
            let stmt = p.parse_type_alias()?;
            Ok(Stmt::ExportDecl(Box::new(stmt)))
        }
        TokenKind::Interface => {
            let stmt = p.parse_interface()?;
            Ok(Stmt::ExportDecl(Box::new(stmt)))
        }
        TokenKind::Enum => {
            let stmt = p.parse_enum()?;
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
