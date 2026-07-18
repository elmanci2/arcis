//! Hover provider.
//!
//! Pure function: given the text up to the cursor and the text after
//! it, returns the [`crate::lsp::Hover`] for the identifier under the
//! cursor. Looks the identifier up in [`crate::builtins`].
//!
//! We extract the bare identifier under the cursor (just the token;
//! not the `sys.env.foo` chain). For chained references like
//! `sys.readFile` we look up the whole label.

use crate::lsp::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range};

use crate::builtins;

/// Try to find a hover for the identifier at the cursor.
///
/// `before` is the text on the cursor line *up to* the cursor.
/// `after`  is the text on the cursor line *starting at* the cursor.
pub fn hover_at(before: &str, after: &str) -> Option<Hover> {
    let ident = identifier_at(before, after)?;
    let label = chain_label_at(before, after).unwrap_or_else(|| ident.clone());

    let b = builtins::find_by_label(&label)
        .or_else(|| builtins::find_by_label(&ident))?;

    let body = format!(
        "```arcis\n{}\n```\n\n_{}_\n\n{}",
        b.detail,
        kind_label(b.kind),
        b.documentation,
    );

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: body,
        }),
        range: Some(identifier_range(before, after)),
    })
}

/// Return the identifier token under the cursor, if any.
fn identifier_at(before: &str, after: &str) -> Option<String> {
    // Walk back from cursor to find the start of the identifier.
    let prefix_start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident_cont(*c))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(before.len());
    let prefix = &before[prefix_start..];

    // Walk forward from cursor to find the end of the identifier.
    let suffix_end = after
        .char_indices()
        .take_while(|(_, c)| is_ident_cont(*c))
        .last()
        .map(|(i, _)| i + c_len_after(&after, i))
        .unwrap_or(0);
    let suffix = &after[..suffix_end];

    let combined = format!("{prefix}{suffix}");
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

/// Find the byte length of one char at position `i` in `s`.
fn c_len_after(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(1, |c| c.len_utf8())
}

/// If the cursor is on a chain like `sys.readFile`, return the whole
/// `sys.readFile` label.
fn chain_label_at(before: &str, after: &str) -> Option<String> {
    // We accumulate segments left-to-right and join with `.`.
    let mut segments: Vec<&str> = Vec::new();

    // 1. The partial identifier that crosses the cursor. `partial_end`
    //    is the position right after any trailing ident chars in
    //    `before`; `cut` is the position of the ident's start.
    let mut cut = before.len();
    let partial_end = cut;
    while cut > 0 && is_ident_cont(before[cut - 1..].chars().next()?) {
        cut -= 1;
    }
    if cut < partial_end {
        segments.push(&before[cut..partial_end]);
    }

    // 2. Walk back through `.ident` segments.
    while cut > 0 && before.as_bytes().get(cut - 1) == Some(&b'.') {
        cut -= 1; // skip the dot
        let id_end = cut;
        while cut > 0 {
            let prev = before[..cut].chars().rev().next()?;
            if !is_ident_start(prev) {
                break;
            }
            cut -= prev.len_utf8();
        }
        if cut == id_end {
            // No identifier after the dot — bail.
            break;
        }
        segments.push(&before[cut..id_end]);
    }

    // `segments` is in reverse order; reverse to get left-to-right.
    segments.reverse();

    // 3. Append the suffix (the leading identifier of `after`, if any).
    let suffix_end = after
        .char_indices()
        .take_while(|(_, c)| is_ident_cont(*c))
        .last()
        .map(|(i, _)| i + c_len_after(after, i))
        .unwrap_or(0);
    let suffix = &after[..suffix_end];

    let joined = segments.join(".");
    let full = format!("{joined}{suffix}");
    if full.is_empty() {
        None
    } else {
        Some(full)
    }
}

/// Compute the LSP `Range` covering the identifier under the cursor.
fn identifier_range(before: &str, after: &str) -> Range {
    let line = before.matches('\n').count() as u32;
    let col_before = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let prefix_ident_len = before[col_before..]
        .chars()
        .rev()
        .take_while(|c| is_ident_cont(*c))
        .count();
    let col_start = (before[col_before..].chars().count() - prefix_ident_len) as u32;
    let suffix_len = after
        .chars()
        .take_while(|c| is_ident_cont(*c))
        .count() as u32;
    Range {
        start: Position {
            line,
            character: col_start,
        },
        end: Position {
            line,
            character: col_start + prefix_ident_len as u32 + suffix_len,
        },
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Friendly label for the hover panel header.
fn kind_label(k: crate::lsp::CompletionItemKind) -> &'static str {
    let kf = crate::lsp::CompletionItemKind::FUNCTION;
    let km = crate::lsp::CompletionItemKind::METHOD;
    let kp = crate::lsp::CompletionItemKind::PROPERTY;
    let kk = crate::lsp::CompletionItemKind::KEYWORD;
    let kmod = crate::lsp::CompletionItemKind::MODULE;
    if k == kf {
        "function"
    } else if k == km {
        "method"
    } else if k == kp {
        "property"
    } else if k == kk {
        "keyword"
    } else if k == kmod {
        "module"
    } else {
        "builtin"
    }
}