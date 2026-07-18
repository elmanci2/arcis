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

/// Walk back from the cursor collecting the `.<identifier>` chain.
/// Returns the chain in left-to-right order (closest-to-cursor
/// segment first). Empty if the character before the cursor is not
/// `.` or `<identifier>`.
fn chain_before_cursor(text: &str) -> Vec<String> {
    let mut rev_chain = Vec::new();
    let bytes = text.as_bytes();
    let mut i = bytes.len();

    // Skip any trailing identifier chars (the partial token the
    // user is typing).
    while i > 0 && is_ident_cont(bytes[i - 1]) {
        i -= 1;
    }

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

    // Reverse to get left-to-right order: e.g. for `sys.env.` the
    // collector saw `env`, `sys`, and we want `sys`, `env`.
    rev_chain.reverse();
    rev_chain
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Compute the completion items to offer at the given cursor.
pub fn completions_at(text_before_cursor: &str) -> Vec<CompletionItem> {
    let chain = chain_before_cursor(text_before_cursor);
    // Convert to `Vec<&str>` so we can match against string literals.
    let chain: Vec<&str> = chain.iter().map(String::as_str).collect();

    match chain.as_slice() {
        // Bare identifier or empty → keywords + print/input + sys.* top-level.
        [] => {
            let mut v: Vec<&Builtin> = builtins::KEYWORDS.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        // `sys.` → namespaces + top-level sys.*.
        ["sys"] => {
            let mut v: Vec<&Builtin> = builtins::SYS_NAMESPACES.iter().collect();
            v.extend(builtins::TOP_LEVEL_BUILTINS.iter());
            v.iter().map(|b| to_item(b)).collect()
        }
        // `sys.<ns>.` → methods of <ns>.
        ["sys", ns] => builtins::ns_methods(ns)
            .iter()
            .map(|b| to_item(b))
            .collect(),
        // `<known-ns>.` (without `sys` prefix) — offer top-level sys.*
        // since `disk.foo` is invalid; the user probably meant
        // `sys.disk.foo`.
        [ns] if is_known_ns(ns) => builtins::TOP_LEVEL_BUILTINS
            .iter()
            .map(|b| to_item(b))
            .collect(),
        // `<id>.` (some other identifier, possibly a variable) →
        // array + string methods.
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
