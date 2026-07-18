//! Completion provider.
//!
//! Pure function: given the text of the document up to the cursor
//! position, returns the list of [`crate::lsp::CompletionItem`]s
//! that should be offered.
//!
//! ## Context detection
//!
//! We look at the trailing dot-chain before the cursor:
//!
//! | Trailing chain | What we offer |
//! |----------------|----------------|
//! | (none)         | keywords + `print` / `input` + top-level `sys.*` |
//! | `sys.`         | namespace names + top-level `sys.*` |
//! | `sys.<ns>.`    | the methods of `<ns>` |
//! | `<id>.`        | array + string method chains |
//! | `<known-ns>.`  | top-level `sys.*` (since `disk.foo` is invalid; user meant `sys.disk`) |
//!
//! Note that completion can only see the chain immediately to the left
//! of the cursor — deeper context (e.g. "is the receiver here a string
//! or an array?") is left for a future pass. Today we always offer the
//! union of array and string methods whenever we see `<id>.`.

use crate::builtins::{self, Builtin};
use crate::lsp::{CompletionItem, InsertTextFormat};

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
pub fn completions_at(text_before_cursor: &str) -> Vec<CompletionItem> {
    let (chain, partial) = chain_before_cursor(text_before_cursor);
    // Convert to `Vec<&str>` so we can match against string literals.
    let chain: Vec<&str> = chain.iter().map(String::as_str).collect();
    // Drop the partial from the chain view — we only match on the
    // already-typed segments. The partial itself doesn't change
    // which slice of the table to serve; it just means the user is
    // mid-typing and the editor will filter by `filterText` anyway.
    let _ = partial;

    match chain.as_slice() {
        // Empty chain (cursor at start, or just after `.`, or just
        // mid-typing a bare ident) → keywords + print/input + sys.*
        // top-level.
        [] => {
            let mut v: Vec<&Builtin> = builtins::KEYWORDS.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        // `sys` (with or without trailing `.`) → namespaces + top-level
        // sys.* builtins.
        ["sys"] => {
            let mut v: Vec<&Builtin> = builtins::SYS_NAMESPACES.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        // `sys.<ns>` (with or without trailing `.`) → methods of <ns>.
        ["sys", ns] if is_known_ns(ns) => builtins::ns_methods(ns)
            .iter()
            .map(|b| to_item(b))
            .collect(),
        // `<known-ns>` (no `sys` prefix, no trailing `.`) — typo
        // recovery: offer top-level sys.* builtins.
        [ns] if is_known_ns(ns) => builtins::TOP_LEVEL_BUILTINS
            .iter()
            .map(|b| to_item(b))
            .collect(),
        // `<id>.` — user typed an identifier followed by `.`; offer
        // array + string method chains.
        [_] => {
            let mut v: Vec<&Builtin> =
                builtins::ARRAY_METHODS.iter().collect();
            v.extend(builtins::STRING_METHODS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        // Anything deeper → empty.
        _ => Vec::new(),
    }
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
