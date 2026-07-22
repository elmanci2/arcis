//! Completion provider.
//!
//! Pure function: given the text of the document up to the cursor
//! position and the full document text, returns the list of
//! [`crate::lsp::CompletionItem`]s that should be offered.
//!
//! ## Context detection
//!
//! We look at the trailing dot-chain before the cursor:
//!
//! | Trailing chain | What we offer |
//! |----------------|----------------|
//! | (none)         | keywords + `print` / `input` + top-level `sys.*` + locals + imports |
//! | `sys.`         | namespace names + top-level `sys.*` |
//! | `sys.<ns>.`    | the methods of `<ns>` |
//! | `<id>.`        | array + string method chains |
//! | `<known-ns>.`  | top-level `sys.*` (since `disk.foo` is invalid; user meant `sys.disk`) |
//!
//! When the cursor is at the start of a statement (no dot-chain),
//! we also include locally-defined names — `let`/`const` bindings,
//! functions, parameters, `for-of` loop variables, and imported
//! symbols — parsed from the full document.

use arcis_ast::{ExportDefault, Function, Stmt};
use arcis_lexer::lex;
use arcis_parser::parse;

use crate::builtins::{self, Builtin};
use crate::lsp::{CompletionItem, CompletionItemKind, InsertTextFormat};

/// Build the completion items for a single entry in the static table.
fn to_item(b: &Builtin) -> CompletionItem {
    CompletionItem {
        label: b.label.to_string(),
        kind: Some(b.kind),
        detail: Some(b.detail.to_string()),
        documentation: Some(crate::lsp::Documentation::MarkupContent(
            crate::lsp::MarkupContent {
                kind: crate::lsp::MarkupKind::Markdown,
                value: b.documentation.to_string(),
            },
        )),
        // For FUNCTION/METHOD labels, insert `name(` so the user
        // gets the opening parenthesis for free.
        insert_text: if matches!(b.kind, crate::lsp::CompletionItemKind::FUNCTION)
            || matches!(b.kind, crate::lsp::CompletionItemKind::METHOD)
        {
            Some(format!("{}(", b.label))
        } else {
            Some(b.label.to_string())
        },
        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
        text_edit: None,
        filter_text: Some(b.label.to_string()),
        sort_text: Some(b.label.to_string()),
        commit_characters: None,
        ..Default::default()
    }
}

/// Walk back from the cursor collecting the `.ident` chain.
/// Returns `(segments, partial)` where:
///
/// - `segments` is the chain of fully-typed identifiers in
///   left-to-right order. Always ends at a `.` (or is empty).
/// - `partial` is the partial identifier the user is currently
///   typing after the last `.` (or at the start of the line).
///
/// Examples:
///
/// - `"pri"`             → `([], Some("pri"))`
/// - `"myvar."`          → `(["myvar"], None)`
/// - `"sys."`            → `(["sys"], None)`
/// - `"sys.re"`          → `(["sys"], Some("re"))`
/// - `"sys.env.g"`       → `(["sys", "env"], Some("g"))`
fn chain_before_cursor(text: &str) -> (Vec<String>, Option<String>) {
    let mut rev_chain = Vec::new();
    let bytes = text.as_bytes();
    let mut i = bytes.len();

    // 1. Walk back through the trailing identifier (the partial
    //    token the user is typing).
    let partial_start = i;
    while i > 0 && is_ident_cont(bytes[i - 1]) {
        i -= 1;
    }
    let partial = if i < partial_start {
        Some(text[i..partial_start].to_string())
    } else {
        None
    };

    // 2. Walk back through `.ident` segments.
    while i > 0 {
        if bytes[i - 1] == b'.' {
            i -= 1;
            let start = i;
            while i > 0 && is_ident_start(bytes[i - 1]) {
                i -= 1;
            }
            if start == i {
                break;
            }
            rev_chain.push(text[i..start].to_string());
        } else {
            break;
        }
    }

    rev_chain.reverse();
    (rev_chain, partial)
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Compute the completion items to offer at the given cursor.
///
/// `text_before_cursor` is the document text up to the cursor position;
/// `full_text` is the entire document (used to collect local definitions
/// and imports).
pub fn completions_at(text_before_cursor: &str, full_text: &str) -> Vec<CompletionItem> {
    let (chain, _partial) = chain_before_cursor(text_before_cursor);
    let chain: Vec<&str> = chain.iter().map(String::as_str).collect();

    let mut items: Vec<CompletionItem> = match chain.as_slice() {
        // Empty chain → keywords + top-level builtins + local defs.
        [] => {
            let mut v: Vec<&Builtin> = builtins::KEYWORDS.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            let mut items: Vec<CompletionItem> =
                v.iter().map(|b| to_item(b)).collect();
            // Add local variables, functions, and imports.
            items.extend(local_completions(full_text));
            items
        }
        // `sys` → namespaces + top-level sys.* builtins.
        ["sys"] => {
            let mut v: Vec<&Builtin> = builtins::SYS_NAMESPACES.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        // `sys.<ns>` → methods of <ns>.
        ["sys", ns] if is_known_ns(ns) => builtins::ns_methods(ns)
            .iter()
            .map(|b| to_item(b))
            .collect(),
        // `<known-ns>` — typo recovery: offer top-level sys.* builtins.
        [ns] if is_known_ns(ns) => builtins::TOP_LEVEL_BUILTINS
            .iter()
            .map(|b| to_item(b))
            .collect(),
        // `<id>.` — array + string method chains.
        [_] => {
            let mut v: Vec<&Builtin> =
                builtins::ARRAY_METHODS.iter().collect();
            v.extend(builtins::STRING_METHODS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        _ => Vec::new(),
    };

    // Deduplicate by label: local defs take priority over builtins
    // (they appear first), so we keep the first occurrence.
    items.dedup_by(|a, b| a.label == b.label);

    items
}

/// `true` if `name` is a recognised top-level namespace under `sys.`.
fn is_known_ns(name: &str) -> bool {
    matches!(
        name,
        "env" | "os"
            | "memory"
            | "cpu"
            | "gpu"
            | "disk"
            | "net"
            | "process"
            | "fs"
            | "path"
            | "proc_env"
    )
}

// ── Local-definition completion ─────────────────────────────────────────

/// One local name for completion, harvested from the AST.
struct LocalDef {
    name: String,
    kind: CompletionItemKind,
    detail: String,
}

/// Parse `full_text` and collect every named definition (variables,
/// functions, imports, parameters) as completion items.
fn local_completions(full_text: &str) -> Vec<CompletionItem> {
    let tokens = match lex(full_text) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let program = match parse(tokens) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut defs: Vec<LocalDef> = Vec::new();
    for stmt in &program.stmts {
        collect_local_defs(stmt, &mut defs);
    }

    // Deduplicate by name: first definition wins.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    defs.retain(|d| seen.insert(d.name.clone()));

    defs.iter()
        .map(|d| CompletionItem {
            label: d.name.clone(),
            kind: Some(d.kind),
            detail: Some(d.detail.clone()),
            filter_text: Some(d.name.clone()),
            sort_text: Some(d.name.clone()),
            ..Default::default()
        })
        .collect()
}

fn collect_local_defs(stmt: &Stmt, defs: &mut Vec<LocalDef>) {
    match stmt {
        // ── let / const ────────────────────────────────────────────
        Stmt::Let { name, ty, .. } => {
            let detail = if let Some(t) = ty {
                format!("{} (variable)", type_label(t))
            } else {
                "variable".to_string()
            };
            defs.push(LocalDef { name: name.clone(), kind: CompletionItemKind::VARIABLE, detail });
        }
        Stmt::Const { name, ty, .. } => {
            let detail = if let Some(t) = ty {
                format!("{} (constant)", type_label(t))
            } else {
                "constant".to_string()
            };
            defs.push(LocalDef { name: name.clone(), kind: CompletionItemKind::VARIABLE, detail });
        }

        // ── function ──────────────────────────────────────────────
        Stmt::Function(f) => {
            defs.push(LocalDef {
                name: f.name.clone(),
                kind: CompletionItemKind::FUNCTION,
                detail: func_sig(f),
            });
            // Parameters
            for p in &f.params {
                defs.push(LocalDef {
                    name: p.name.clone(),
                    kind: CompletionItemKind::VARIABLE,
                    detail: format!("{} (parameter)", type_label(&p.ty)),
                });
            }
            // Recurse into body
            for s in &f.body {
                collect_local_defs(s, defs);
            }
        }

        // ── for-of ────────────────────────────────────────────────
        Stmt::ForOf { name, body, .. } => {
            defs.push(LocalDef {
                name: name.clone(),
                kind: CompletionItemKind::VARIABLE,
                detail: "loop variable".to_string(),
            });
            for s in body {
                collect_local_defs(s, defs);
            }
        }

        // ── imports ───────────────────────────────────────────────
        Stmt::Import { module, alias } => {
            let local = alias.clone().unwrap_or_else(|| {
                module.last().cloned().unwrap_or_default()
            });
            defs.push(LocalDef {
                name: local,
                kind: CompletionItemKind::MODULE,
                detail: format!("module \"{}\"", module.join(".")),
            });
        }
        Stmt::FromImport { module, names, wildcard: false } => {
            for n in names {
                let local = n.alias.clone().unwrap_or_else(|| n.name.clone());
                defs.push(LocalDef {
                    name: local,
                    kind: CompletionItemKind::VARIABLE,
                    detail: format!("from \"{}\"", module.join(".")),
                });
            }
        }

        // ── export decl (inline) ──────────────────────────────────
        Stmt::ExportDecl(inner) => match inner.as_ref() {
            Stmt::Let { name, ty, .. } => {
                let detail = if let Some(t) = ty {
                    format!("{} (exported)", type_label(t))
                } else {
                    "exported".to_string()
                };
                defs.push(LocalDef { name: name.clone(), kind: CompletionItemKind::VARIABLE, detail });
            }
            Stmt::Const { name, ty, .. } => {
                let detail = if let Some(t) = ty {
                    format!("{} (exported constant)", type_label(t))
                } else {
                    "exported constant".to_string()
                };
                defs.push(LocalDef { name: name.clone(), kind: CompletionItemKind::VARIABLE, detail });
            }
            Stmt::Function(f) => {
                defs.push(LocalDef {
                    name: f.name.clone(),
                    kind: CompletionItemKind::FUNCTION,
                    detail: format!("{} (exported)", func_sig(f)),
                });
                for p in &f.params {
                    defs.push(LocalDef {
                        name: p.name.clone(),
                        kind: CompletionItemKind::VARIABLE,
                        detail: format!("{} (parameter)", type_label(&p.ty)),
                    });
                }
                for s in &f.body {
                    collect_local_defs(s, defs);
                }
            }
            _ => {}
        },

        // ── export spec / export default ──────────────────────────
        Stmt::ExportSpec(items) => {
            for item in items {
                let name = item.alias.clone().unwrap_or_else(|| item.name.clone());
                defs.push(LocalDef {
                    name,
                    kind: CompletionItemKind::VARIABLE,
                    detail: "re-export".to_string(),
                });
            }
        }
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            let name = if f.name.is_empty() {
                "default".to_string()
            } else {
                f.name.clone()
            };
            defs.push(LocalDef {
                name,
                kind: CompletionItemKind::FUNCTION,
                detail: format!("{} (default export)", func_sig(f)),
            });
            for s in &f.body {
                collect_local_defs(s, defs);
            }
        }
        Stmt::ExportDefault(ExportDefault::Expr(_)) => {
            defs.push(LocalDef {
                name: "default".to_string(),
                kind: CompletionItemKind::VARIABLE,
                detail: "default export".to_string(),
            });
        }

        // ── compound statements: recurse ──────────────────────────
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_local_defs(s, defs);
            }
            if let Some(els) = else_branch {
                for s in els {
                    collect_local_defs(s, defs);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in body {
                collect_local_defs(s, defs);
            }
        }
        Stmt::For { init, body, .. } => {
            if let Some(init_stmt) = init {
                collect_local_defs(init_stmt, defs);
            }
            for s in body {
                collect_local_defs(s, defs);
            }
        }

        // ── leaves ────────────────────────────────────────────────
        Stmt::Assign { .. }
        | Stmt::AssignIndex { .. }
        | Stmt::AssignMember { .. }
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Expr(_)
        | Stmt::FromImport { wildcard: true, .. } => {}
    }
}

/// Human-readable label for a type annotation.
fn type_label(ty: &arcis_ast::Type) -> String {
    if ty.is_array {
        format!("{}[]", ty.name)
    } else if ty.fields.is_empty() {
        ty.name.clone()
    } else {
        let fields: Vec<String> = ty
            .fields
            .iter()
            .map(|(n, t)| format!("{n}: {}", type_label(t)))
            .collect();
        format!("{{ {} }}", fields.join(", "))
    }
}

/// Build a short function signature string like `(a: number, b: number) -> number`.
fn func_sig(f: &Function) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, type_label(&p.ty)))
        .collect();
    format!("({}) -> {}", params.join(", "), type_label(&f.return_type))
}
