//! Diagnostics provider.
//!
//! Pure function: takes the full document text, returns a list of
//! [`crate::lsp::Diagnostic`]s by running:
//!
//! 1. `arcis_lexer::lex` — surfaces lexical errors.
//! 2. `arcis_parser::parse` — surfaces syntax errors.
//!
//! Validation (unused / duplicate) is left out for now: the current
//! `arcis-validation` crate reports issues without per-symbol line
//! info compatible with LSP `Range`. Wiring it in cleanly is a
//! follow-up that depends on the validator's structured output
//! stabilising.

use arcis_parser::ParseError;
use crate::lsp::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Run the diagnostic pipeline against `text` and return every
/// issue we can identify.
pub fn diagnostics_for(text: &str) -> Vec<Diagnostic> {
    // 1. Lex
    let tokens = match arcis_lexer::lex(text) {
        Ok(t) => t,
        Err(_e) => {
            // LexError carries a `Display` impl but no line info
            // we can wire into a Range. Report it as a whole-document
            // diagnostic; a future improvement is to thread the
            // token stream through and report the bad token's span.
            return vec![full_document_diagnostic(
                "lex error: couldn't tokenise the document",
                DiagnosticSeverity::ERROR,
            )];
        }
    };

    // 2. Parse
    match arcis_parser::parse(tokens) {
        Ok(_program) => vec![],
        Err(e) => vec![from_parse(e)],
    }
}

fn from_parse(e: ParseError) -> Diagnostic {
    // ParseError is 1-indexed for line+col; LSP is 0-indexed.
    let line = e.line.saturating_sub(1) as u32;
    let character = e.col.saturating_sub(1) as u32;
    Diagnostic {
        range: Range {
            start: Position { line, character },
            end: Position {
                line,
                character: character.saturating_add(1),
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("arcis-lsp".into()),
        message: e.msg,
        ..Default::default()
    }
}

fn full_document_diagnostic(message: &str, severity: DiagnosticSeverity) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: u32::MAX,
                character: u32::MAX,
            },
        },
        severity: Some(severity),
        source: Some("arcis-lsp".into()),
        message: message.to_string(),
        ..Default::default()
    }
}
