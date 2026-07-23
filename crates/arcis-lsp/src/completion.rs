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
//! | Trailing chain    | What we offer |
//! |--------------------|----------------|
//! | (none)              | keywords + `print` / `input` + top-level `sys.*` + locals + imports |
//! | `sys.`              | namespace names + top-level `sys.*` |
//! | `sys.<ns>.`         | the methods of `<ns>` |
//! | `<enum-name>.`      | the enum's variants (e.g. `Color.` → `Red`, `Green`, …) |
//! | `<id>.`             | array + string method chains |
//! | `<known-ns>.`       | top-level `sys.*` (since `disk.foo` is invalid; user meant `sys.disk`) |
//!
//! When the cursor is at the start of a statement (no dot-chain), we
//! also include every name from [`crate::symbols::collect_symbols`] —
//! `let`/`const` bindings, functions, parameters, loop variables,
//! imports, type aliases, interfaces, and enums — parsed from the full
//! document.

use arcis_lexer::lex;
use arcis_parser::parse;

use crate::builtins::{self, Builtin};
use crate::lsp::{CompletionItem, InsertTextFormat};
use crate::symbols::{self, Symbol, SymbolKind};

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

/// Build a completion item for a locally-declared symbol.
fn to_symbol_item(s: &Symbol) -> CompletionItem {
    CompletionItem {
        label: s.name.clone(),
        kind: Some(s.kind.completion_kind()),
        detail: Some(s.detail.clone()),
        filter_text: Some(s.name.clone()),
        sort_text: Some(s.name.clone()),
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

    let local_symbols = parse_symbols(full_text);

    let mut items: Vec<CompletionItem> = match chain.as_slice() {
        // Empty chain → keywords + top-level builtins + local defs.
        [] => {
            let mut v: Vec<&Builtin> = builtins::KEYWORDS.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            let mut items: Vec<CompletionItem> = v.iter().map(|b| to_item(b)).collect();
            items.extend(local_symbols.iter().map(to_symbol_item));
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
        // `<enum-name>` — a locally-declared enum → offer its variants.
        [name] if enum_variants(&local_symbols, name).is_some() => enum_variants(&local_symbols, name)
            .unwrap()
            .iter()
            .map(|(variant, value)| CompletionItem {
                label: variant.clone(),
                kind: Some(crate::lsp::CompletionItemKind::ENUM_MEMBER),
                detail: Some(format!("{name}.{variant} = {value}")),
                filter_text: Some(variant.clone()),
                sort_text: Some(variant.clone()),
                ..Default::default()
            })
            .collect(),
        // `<known-ns>` — typo recovery: offer top-level sys.* builtins.
        [ns] if is_known_ns(ns) => builtins::TOP_LEVEL_BUILTINS
            .iter()
            .map(|b| to_item(b))
            .collect(),
        // `<id>.` — array + string method chains.
        [_] => {
            let mut v: Vec<&Builtin> = builtins::ARRAY_METHODS.iter().collect();
            v.extend(builtins::STRING_METHODS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        _ => Vec::new(),
    };

    // Deduplicate by label: local defs / enum variants take priority
    // over builtins (they appear first), so we keep the first
    // occurrence.
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

/// Parse `full_text` into the shared symbol table, with inferred types on
/// unannotated bindings. Mid-edit syntax errors degrade gracefully: the
/// broken line is blanked and the rest of the file still completes; a
/// hopeless document returns an empty list rather than an error.
fn parse_symbols(full_text: &str) -> Vec<Symbol> {
    match symbols::parse_lenient(full_text) {
        Some(program) => symbols::collect_symbols(&program),
        None => Vec::new(),
    }
}

/// If `name` names a locally-declared enum, return its `(variant,
/// value)` list.
fn enum_variants<'a>(syms: &'a [Symbol], name: &str) -> Option<&'a [(String, i64)]> {
    syms.iter()
        .find(|s| s.kind == SymbolKind::Enum && s.name == name)
        .map(|s| s.enum_variants.as_slice())
}
