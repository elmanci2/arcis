//! Completion provider.
//!
//! Given the text of the document up to the cursor position, the full
//! document text, and (when available) the document's URI, returns the
//! list of [`crate::lsp::CompletionItem`]s that should be offered.
//!
//! ## Context detection
//!
//! We look at the trailing dot-chain before the cursor:
//!
//! | Trailing chain    | What we offer |
//! |--------------------|----------------|
//! | (none)              | keywords + `print` / `input` + top-level `sys.*` + every local/global symbol in the document (+ names pulled in via `from x import *`) |
//! | `sys.`              | namespace names + top-level `sys.*` |
//! | `sys.<ns>.`         | the methods of `<ns>` |
//! | `<enum-name>.`      | the enum's variants (e.g. `Color.` → `Red`, `Green`, …) |
//! | `<namespace-import>.` | the imported module's own exported functions/consts/types (cross-file — see [`cross_module_completions`]) |
//! | `<object-var>.`     | the variable's actual field names (resolved via its declared/inferred type, following `interface`/`type` aliases in the same document) |
//! | `<array-var>.`      | array methods only |
//! | `<string-var>.`     | string methods only |
//! | `<number/boolean-var>.` | nothing (neither array nor string methods apply) |
//! | `<id>.` (type unknown) | array + string method chains — permissive fallback so completion never goes silent just because inference couldn't pin the type down |
//! | `<known-ns>.`       | top-level `sys.*` (since `disk.foo` is invalid; user meant `sys.disk`) |
//!
//! When the cursor is at the start of a statement (no dot-chain), we
//! also include every name from [`crate::symbols::collect_symbols`] —
//! `let`/`const` bindings (function-local AND top-level/"global"),
//! functions, parameters, loop variables, imports, type aliases,
//! interfaces, and enums — parsed from the full document, plus every
//! exported name from any `from x import *;` wildcard import.

use std::collections::HashMap;

use arcis_ast::Type;

use crate::builtins::{self, Builtin};
use crate::lsp::{CompletionItem, InsertTextFormat, Url};
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
/// `full_text` is the entire document (used to collect local/global
/// definitions and imports). `doc_uri`, when available, enables
/// cross-file completion (namespace-import members, `from x import *`) by
/// resolving imports relative to the current file — same resolution
/// `definition::goto_definition` already uses. Pass `None` (e.g. from a
/// standalone/test context with no real file) to skip cross-file lookups;
/// everything else still works.
pub fn completions_at(
    text_before_cursor: &str,
    full_text: &str,
    doc_uri: Option<&Url>,
) -> Vec<CompletionItem> {
    let (chain, _partial) = chain_before_cursor(text_before_cursor);
    let chain: Vec<&str> = chain.iter().map(String::as_str).collect();

    let doc = ParsedDoc::from_text(full_text);

    let mut items: Vec<CompletionItem> = match chain.as_slice() {
        // Empty chain → keywords + top-level builtins + every local/global
        // symbol + wildcard-imported names.
        [] => {
            let mut v: Vec<&Builtin> = builtins::KEYWORDS.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            let mut items: Vec<CompletionItem> = v.iter().map(|b| to_item(b)).collect();
            items.extend(doc.symbols.iter().map(to_symbol_item));
            if let Some(uri) = doc_uri {
                for module in &doc.wildcard_imports {
                    items.extend(cross_module_completions(uri, module));
                }
            }
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
        [name] if enum_variants(&doc.symbols, name).is_some() => enum_variants(&doc.symbols, name)
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
        // `<id>.` — resolve the actual member set from `name`'s declared
        // type when we can; otherwise fall back to the old permissive
        // guess (array + string methods) so completion never goes silent.
        [name] => member_completions(name, &doc, doc_uri)
            .unwrap_or_else(|| {
                let mut v: Vec<&Builtin> = builtins::ARRAY_METHODS.iter().collect();
                v.extend(builtins::STRING_METHODS.iter());
                v.iter().map(|b| to_item(b)).collect()
            }),
        _ => Vec::new(),
    };

    // Deduplicate by label: local defs / enum variants take priority
    // over builtins (they appear first), so we keep the first
    // occurrence.
    items.dedup_by(|a, b| a.label == b.label);

    items
}

/// If `name` resolves to a symbol whose actual member set we can pin
/// down — an object/interface-shaped variable, an array, a string, a
/// definite non-object primitive (which has no members), or a
/// namespace-imported module — return the right completion items.
/// `None` means "couldn't determine" (unknown/inferred-`any`/union type,
/// or `name` isn't a known symbol at all): the caller falls back to the
/// permissive array+string guess rather than offering nothing.
fn member_completions(
    name: &str,
    doc: &ParsedDoc,
    doc_uri: Option<&Url>,
) -> Option<Vec<CompletionItem>> {
    let sym = doc.symbols.iter().find(|s| s.name == name)?;

    // Namespace-imported module (`import utils;` / `import utils as u;`)
    // → the target file's own exported members, not array/string guesses.
    if sym.kind == SymbolKind::Module {
        let (module, _) = sym.import.as_ref()?;
        return Some(cross_module_completions(doc_uri?, module));
    }

    member_completions_for_type(sym.ty.as_ref()?, &doc.named_shapes)
}

fn member_completions_for_type(
    ty: &Type,
    named_shapes: &HashMap<String, Vec<(String, Type, bool)>>,
) -> Option<Vec<CompletionItem>> {
    match ty {
        Type::Optional(inner) => member_completions_for_type(inner, named_shapes),
        Type::Object { fields, .. } => {
            let owned: Vec<(String, Type, bool)> = fields
                .iter()
                .map(|(n, t, opt)| (n.clone(), (**t).clone(), *opt))
                .collect();
            Some(object_field_items(&owned))
        }
        // A bare interface/type-alias reference — look up its fields
        // among this document's own declarations.
        Type::Named(n) => named_shapes.get(n).map(|fields| object_field_items(fields)),
        Type::Array(_) => Some(builtins::ARRAY_METHODS.iter().map(to_item).collect()),
        Type::Primitive(p) if p == "string" => {
            Some(builtins::STRING_METHODS.iter().map(to_item).collect())
        }
        // Definitely no array/string methods on these — offering them
        // would just be noise, unlike the "unknown type" case below.
        Type::Primitive(p) if matches!(p.as_str(), "number" | "boolean" | "void") => {
            Some(Vec::new())
        }
        // `any`, unions, function types, etc. — genuinely don't know.
        _ => None,
    }
}

fn object_field_items(fields: &[(String, Type, bool)]) -> Vec<CompletionItem> {
    fields
        .iter()
        .map(|(name, ty, _optional)| CompletionItem {
            label: name.clone(),
            kind: Some(crate::lsp::CompletionItemKind::FIELD),
            detail: Some(symbols::type_label(ty)),
            filter_text: Some(name.clone()),
            sort_text: Some(name.clone()),
            ..Default::default()
        })
        .collect()
}

/// Resolve a module path (namespace import or `from x import *`) to its
/// exported symbols, by locating and parsing the target `.tsr` file
/// relative to `uri` — the same resolution
/// [`crate::definition::goto_definition`] uses for cross-file jumps.
/// Degrades to an empty list (never an error) when the file can't be
/// found or doesn't currently parse, matching this module's overall
/// philosophy of never surfacing an error from completion.
fn cross_module_completions(uri: &Url, module: &[String]) -> Vec<CompletionItem> {
    let Some(path) = crate::definition::module_file_path(uri, module) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Some(program) = symbols::parse_lenient(&text) else {
        return Vec::new();
    };
    symbols::collect_symbols(&program)
        .into_iter()
        .filter(|s| s.exported)
        .map(|s| to_symbol_item(&s))
        .collect()
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

/// Everything derived from one parse of the document, computed once per
/// completion request and threaded through the dispatch above.
struct ParsedDoc {
    /// Every local/global symbol — `let`/`const` bindings (module-level
    /// AND function-local), functions, parameters, loop variables,
    /// imports, type aliases, interfaces, enums.
    symbols: Vec<Symbol>,
    /// `interface`/`type` name → field list, for resolving a variable's
    /// `Type::Named(...)` annotation to its actual members.
    named_shapes: HashMap<String, Vec<(String, Type, bool)>>,
    /// Module paths pulled in via `from x import *;`, in source order.
    wildcard_imports: Vec<Vec<String>>,
}

impl ParsedDoc {
    /// Parse `full_text` (leniently — a mid-edit syntax error blanks just
    /// the offending line so the rest of the file still completes) and
    /// derive every piece of state completion needs. A hopeless document
    /// (nothing parses even after blanking) yields all-empty fields
    /// rather than an error.
    fn from_text(full_text: &str) -> Self {
        match symbols::parse_lenient(full_text) {
            Some(program) => ParsedDoc {
                symbols: symbols::collect_symbols(&program),
                named_shapes: symbols::collect_named_shapes(&program),
                wildcard_imports: wildcard_import_modules(&program),
            },
            None => ParsedDoc {
                symbols: Vec::new(),
                named_shapes: HashMap::new(),
                wildcard_imports: Vec::new(),
            },
        }
    }
}

/// Every `from x import *;` module path, top-level only (imports are only
/// meaningful there, matching how `symbols.rs` treats them).
fn wildcard_import_modules(program: &arcis_ast::Program) -> Vec<Vec<String>> {
    program
        .stmts
        .iter()
        .filter_map(|s| match s {
            arcis_ast::Stmt::FromImport { module, wildcard: true, .. } => Some(module.clone()),
            _ => None,
        })
        .collect()
}

/// If `name` names a locally-declared enum, return its `(variant,
/// value)` list.
fn enum_variants<'a>(syms: &'a [Symbol], name: &str) -> Option<&'a [(String, i64)]> {
    syms.iter()
        .find(|s| s.kind == SymbolKind::Enum && s.name == name)
        .map(|s| s.enum_variants.as_slice())
}
