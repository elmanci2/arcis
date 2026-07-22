//! Go-to-definition provider.
//!
//! When the user Ctrl+Clicks on an identifier we resolve it to its
//! definition site — either in the same file or across module boundaries.
//!
//! ## Resolution order
//!
//! 1. **Local definitions** — `let`, `const`, `function`, `for-of` loop
//!    variables, and function parameters whose name matches the cursor
//!    identifier.
//! 2. **Imports** — `from <module> import <name>` or
//!    `import <module> as <name>`. The target module file (`.tsr`) is
//!    located relative to the current file, parsed, and searched for the
//!    matching `export`.
//!
//! Cross-file resolution follows Python-style module layout:
//! - `import utils`          → `./utils.tsr`
//! - `import os.path`        → `./os/path.tsr`
//! - `from mate import sumar` → `./mate.tsr`, then find `export … sumar`

use std::fs;
use std::path::PathBuf;

use arcis_ast::{ExportDefault, Function, Program, Stmt};
use arcis_lexer::lex;
use arcis_parser::parse;

use crate::lsp::{GotoDefinitionResponse, Location, Position, Range, Url};

// ── Public entry point ─────────────────────────────────────────────────

/// Try to find the definition of the identifier at `pos` in `text`.
///
/// `uri` is the `file://` URL of the current document; it is used to
/// resolve relative module imports.
pub fn goto_definition(
    text: &str,
    uri: &Url,
    pos: Position,
) -> Option<GotoDefinitionResponse> {
    let ident = identifier_at(text, pos)?;

    // Parse the current document so we can build the definition table.
    let tokens = lex(text).ok()?;
    let program = parse(tokens).ok()?;

    let defs = collect_definitions(&program);

    // 1. Local definition?
    if let Some(d) = defs.iter().find(|d| d.name == ident && d.import.is_none()) {
        return Some(GotoDefinitionResponse::Scalar(location(
            uri,
            d.line,
            d.col,
            d.name.len(),
        )));
    }

    // 2. Imported name?
    if let Some(d) = defs.iter().find(|d| d.name == ident && d.import.is_some()) {
        let (ref module, ref export_name) = d.import.as_ref().unwrap();
        return resolve_import(uri, module, export_name.as_deref());
    }

    None
}

// ── Identifier extraction ──────────────────────────────────────────────

/// Return the identifier token under the cursor, if any.
fn identifier_at(text: &str, pos: Position) -> Option<String> {
    // Walk to the start of the cursor line.
    let mut line_start = 0;
    let mut cur_line: u32 = 0;
    for (i, c) in text.char_indices() {
        if cur_line == pos.line {
            line_start = i;
            break;
        }
        if c == '\n' {
            cur_line += 1;
        }
    }
    // Find the end of the cursor line.
    let line_end = text[line_start..]
        .find('\n')
        .map(|n| line_start + n)
        .unwrap_or(text.len());
    let cursor_line = &text[line_start..line_end];

    let char_idx = pos.character.min(cursor_line.chars().count() as u32) as usize;

    // Walk left to find the start of the identifier.
    let prefix: String = cursor_line.chars().take(char_idx).collect();
    let ident_start = prefix
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident_cont(*c))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(prefix.len());

    // Walk right to find the end of the identifier.
    let suffix: String = cursor_line.chars().skip(char_idx).collect();
    let ident_end = suffix
        .char_indices()
        .take_while(|(_, c)| is_ident_cont(*c))
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);

    let ident = format!(
        "{}{}",
        &prefix[ident_start..],
        &suffix[..ident_end]
    );
    if ident.is_empty() { None } else { Some(ident) }
}

fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// ── Definition collection ──────────────────────────────────────────────

/// One named binding with its source position and optional import info.
struct DefInfo {
    name: String,
    line: usize,
    col: usize,
    /// `Some((module_path, export_name))` when this is an imported name.
    /// `export_name` is `None` for namespace imports (`import foo`).
    import: Option<(Vec<String>, Option<String>)>,
}

/// Walk the AST and collect every named definition.
fn collect_definitions(program: &Program) -> Vec<DefInfo> {
    let mut defs = Vec::new();
    for stmt in &program.stmts {
        collect_stmt(stmt, &mut defs);
    }
    defs
}

fn collect_stmt(stmt: &Stmt, defs: &mut Vec<DefInfo>) {
    match stmt {
        // ── let / const ────────────────────────────────────────────
        Stmt::Let { name, line, col, .. } | Stmt::Const { name, line, col, .. } => {
            defs.push(DefInfo {
                name: name.clone(),
                line: *line,
                col: *col,
                import: None,
            });
        }

        // ── function ──────────────────────────────────────────────
        Stmt::Function(f) => {
            defs.push(DefInfo {
                name: f.name.clone(),
                line: f.line,
                col: f.col,
                import: None,
            });
            // Collect parameters and body definitions.
            collect_function_defs(f, defs);
        }

        // ── for-of ────────────────────────────────────────────────
        Stmt::ForOf { name, body, .. } => {
            // We don't store line/col for ForOf in the AST, so we
            // approximate: the variable is near the start of the
            // statement.  This is good enough for go-to-definition
            // (the user clicks a usage elsewhere).
            // We'll set line/col to 0 and the lookup will still
            // resolve to the same file.
            defs.push(DefInfo {
                name: name.clone(),
                line: 0,
                col: 0,
                import: None,
            });
            for s in body {
                collect_stmt(s, defs);
            }
        }

        // ── imports ───────────────────────────────────────────────
        Stmt::Import { module, alias } => {
            let local_name = alias.clone().unwrap_or_else(|| module.last().cloned().unwrap_or_default());
            defs.push(DefInfo {
                name: local_name,
                line: 0,
                col: 0,
                import: Some((module.clone(), None)),
            });
        }
        Stmt::FromImport { module, names, wildcard } => {
            if *wildcard {
                // `from foo import *` — can't resolve individual names.
                return;
            }
            for n in names {
                let local_name = n.alias.clone().unwrap_or_else(|| n.name.clone());
                defs.push(DefInfo {
                    name: local_name,
                    line: 0,
                    col: 0,
                    import: Some((module.clone(), Some(n.name.clone()))),
                });
            }
        }

        // ── export decl (inline) ──────────────────────────────────
        Stmt::ExportDecl(inner) => {
            match inner.as_ref() {
                Stmt::Let { name, line, col, .. }
                | Stmt::Const { name, line, col, .. } => {
                    defs.push(DefInfo {
                        name: name.clone(),
                        line: *line,
                        col: *col,
                        import: None,
                    });
                }
                Stmt::Function(f) => {
                    defs.push(DefInfo {
                        name: f.name.clone(),
                        line: f.line,
                        col: f.col,
                        import: None,
                    });
                    collect_function_defs(f, defs);
                }
                _ => {}
            }
        }

        // ── export spec / export default ──────────────────────────
        Stmt::ExportSpec(items) => {
            for item in items {
                defs.push(DefInfo {
                    name: item.alias.clone().unwrap_or_else(|| item.name.clone()),
                    line: 0,
                    col: 0,
                    import: None,
                });
            }
        }
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            let name = if f.name.is_empty() {
                "__default".to_string()
            } else {
                f.name.clone()
            };
            defs.push(DefInfo {
                name,
                line: f.line,
                col: f.col,
                import: None,
            });
            if !f.name.is_empty() {
                collect_function_defs(f, defs);
            }
        }

        // ── compound statements: recurse ──────────────────────────
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_stmt(s, defs);
            }
            if let Some(els) = else_branch {
                for s in els {
                    collect_stmt(s, defs);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in body {
                collect_stmt(s, defs);
            }
        }
        Stmt::For { init, body, .. } => {
            if let Some(init_stmt) = init {
                collect_stmt(init_stmt, defs);
            }
            for s in body {
                collect_stmt(s, defs);
            }
        }

        // ── leaves / unhandled ────────────────────────────────────
        Stmt::Assign { .. }
        | Stmt::AssignIndex { .. }
        | Stmt::AssignMember { .. }
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Expr(_)
        | Stmt::ExportDefault(ExportDefault::Expr(_)) => {}
    }
}

/// Collect parameters and recursively walk the function body.
fn collect_function_defs(f: &Function, defs: &mut Vec<DefInfo>) {
    for p in &f.params {
        defs.push(DefInfo {
            name: p.name.clone(),
            line: p.line,
            col: p.col,
            import: None,
        });
    }
    for s in &f.body {
        collect_stmt(s, defs);
    }
}

// ── Cross-file import resolution ───────────────────────────────────────

/// Given a `file://` URI and a module path like `["mate"]` or
/// `["os", "path"]`, return the filesystem path to the `.tsr` file.
fn module_file_path(current_uri: &Url, module: &[String]) -> Option<PathBuf> {
    if current_uri.scheme() != "file" {
        return None;
    }
    let current_path = PathBuf::from(current_uri.path());
    let dir = current_path.parent()?;

    let mut path = dir.to_path_buf();
    // All segments except the last are directories; the last gets `.tsr`.
    for (i, seg) in module.iter().enumerate() {
        if i == module.len() - 1 {
            path.push(format!("{seg}.tsr"));
        } else {
            path.push(seg);
        }
    }
    Some(path)
}

/// Resolve an import: locate the module file, parse it, and find the
/// matching export.
fn resolve_import(
    current_uri: &Url,
    module: &[String],
    export_name: Option<&str>,
) -> Option<GotoDefinitionResponse> {
    let file_path = module_file_path(current_uri, module)?;
    let text = fs::read_to_string(&file_path).ok()?;
    let tokens = lex(&text).ok()?;
    let program = parse(tokens).ok()?;

    let target_uri = Url::from_file_path(&file_path).ok()?;

    match export_name {
        // `from foo import name` — look for the specific export.
        Some(name) => find_export(&program, name, &target_uri),
        // `import foo` — point to the first export or the file itself.
        None => {
            // Point to the first declaration in the module.
            first_definition_location(&program, &target_uri)
        }
    }
}

/// Search the module's AST for an export whose exported name matches
/// `target_name`.
fn find_export(
    program: &Program,
    target_name: &str,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    for stmt in &program.stmts {
        match stmt {
            // `export function f()` or `export const X` or `export let Y`
            Stmt::ExportDecl(inner) => match inner.as_ref() {
                Stmt::Let { name, line, col, .. }
                | Stmt::Const { name, line, col, .. }
                    if name == target_name =>
                {
                    return Some(GotoDefinitionResponse::Scalar(location(
                        uri, *line, *col, name.len(),
                    )));
                }
                Stmt::Function(f) if f.name == target_name => {
                    return Some(GotoDefinitionResponse::Scalar(location(
                        uri, f.line, f.col, f.name.len(),
                    )));
                }
                _ => {}
            },

            // `export { name, name as alias }`
            Stmt::ExportSpec(items) => {
                for item in items {
                    let exported_name = item.alias.as_ref().unwrap_or(&item.name);
                    if exported_name == target_name {
                        // The re-export points to a local name declared
                        // elsewhere in the same file. Find that local
                        // definition.
                        let local = &item.name;
                        if let Some(loc) =
                            find_local_def(program, local, uri)
                        {
                            return Some(loc);
                        }
                        // Fallback: point to the export spec itself
                        // (line/col 0).
                        return Some(GotoDefinitionResponse::Scalar(Location {
                            uri: uri.clone(),
                            range: Range {
                                start: Position { line: 0, character: 0 },
                                end: Position { line: 0, character: 0 },
                            },
                        }));
                    }
                }
            }

            // `export default function name(){}` or `export default expr`
            Stmt::ExportDefault(export_default) => {
                let default_name = match export_default {
                    ExportDefault::Function(f) => {
                        if f.name.is_empty() { "__default" } else { &f.name }
                    }
                    ExportDefault::Expr(_) => "__default",
                };
                if target_name == default_name || target_name == "__default" {
                    match export_default {
                        ExportDefault::Function(f) => {
                            return Some(GotoDefinitionResponse::Scalar(location(
                                uri, f.line, f.col, f.name.len().max(8),
                            )));
                        }
                        ExportDefault::Expr(_) => {
                            return Some(GotoDefinitionResponse::Scalar(Location {
                                uri: uri.clone(),
                                range: Range {
                                    start: Position { line: 0, character: 0 },
                                    end: Position { line: 0, character: 0 },
                                },
                            }));
                        }
                    }
                }
            }

            _ => {}
        }
    }
    None
}

/// Find a local (non-export) definition by name in a parsed module.
fn find_local_def(
    program: &Program,
    name: &str,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    for stmt in &program.stmts {
        match stmt {
            Stmt::Let { name: n, line, col, .. }
            | Stmt::Const { name: n, line, col, .. }
                if n == name =>
            {
                return Some(GotoDefinitionResponse::Scalar(location(
                    uri, *line, *col, n.len(),
                )));
            }
            Stmt::Function(f) if f.name == name => {
                return Some(GotoDefinitionResponse::Scalar(location(
                    uri, f.line, f.col, f.name.len(),
                )));
            }
            _ => {}
        }
    }
    None
}

/// Return the location of the first definition in the module (used as a
/// fallback for namespace imports like `import foo`).
fn first_definition_location(
    program: &Program,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    for stmt in &program.stmts {
        match stmt {
            Stmt::Let { name, line, col, .. }
            | Stmt::Const { name, line, col, .. } =>
            {
                return Some(GotoDefinitionResponse::Scalar(location(
                    uri, *line, *col, name.len(),
                )));
            }
            Stmt::Function(f) => {
                return Some(GotoDefinitionResponse::Scalar(location(
                    uri, f.line, f.col, f.name.len(),
                )));
            }
            Stmt::ExportDecl(inner) => match inner.as_ref() {
                Stmt::Let { name, line, col, .. }
                | Stmt::Const { name, line, col, .. } =>
                {
                    return Some(GotoDefinitionResponse::Scalar(location(
                        uri, *line, *col, name.len(),
                    )));
                }
                Stmt::Function(f) => {
                    return Some(GotoDefinitionResponse::Scalar(location(
                        uri, f.line, f.col, f.name.len(),
                    )));
                }
                _ => {}
            },
            Stmt::ExportDefault(ExportDefault::Function(f)) => {
                return Some(GotoDefinitionResponse::Scalar(location(
                    uri,
                    f.line,
                    f.col,
                    if f.name.is_empty() { 8 } else { f.name.len() },
                )));
            }
            _ => {}
        }
    }
    None
}

// ── Helpers ────────────────────────────────────────────────────────────

fn location(uri: &Url, line: usize, col: usize, len: usize) -> Location {
    Location {
        uri: uri.clone(),
        range: Range {
            start: Position {
                line: line as u32,
                character: col as u32,
            },
            end: Position {
                line: line as u32,
                character: (col + len) as u32,
            },
        },
    }
}
