//! Diagnostics provider.
//!
//! Pure function: takes the full document text, returns a list of
//! [`crate::lsp::Diagnostic`]s by running:
//!
//! 1. `arcis_lexer::lex` — surfaces lexical errors.
//! 2. `arcis_parser::parse` — surfaces syntax errors.
//! 3. `arcis_validation::validate` — unused variables (warning),
//!    duplicate declarations and `break`/`continue` outside a loop
//!    (errors), each anchored at its declaration's line/col.
//! 4. Type inference + null-safety — same guarantee the compiler enforces
//!    (every `T?` must be resolved with `?? fallback`, a null check, or
//!    `!` before reaching a place that expects a `T`), surfaced live in the
//!    editor rather than only at `arcis build`. Single-document only (the
//!    LSP doesn't resolve cross-file imports), so a name defined in
//!    another module of the same project won't be seen here — that still
//!    gets a definitive answer at build time.

use arcis_parser::ParseError;
use arcis_validation::{NullSafetyIssue, ValidationIssue};
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
    let mut program = match arcis_parser::parse(tokens) {
        Ok(program) => program,
        Err(e) => return vec![from_parse(e)],
    };

    // 3. Validate
    let mut out: Vec<Diagnostic> = arcis_validation::validate(&program)
        .into_iter()
        .map(from_validation)
        .collect();

    // 4. Type inference + null-safety, mirroring the compiler's own
    // pipeline (see `arcis-driver/src/build.rs`) for this one document.
    let mut env = arcis_validation::TypeEnv::default();
    env.add_program(&program);
    arcis_validation::infer_program(&mut program, &env);
    arcis_validation::narrow_program(&mut program);
    out.extend(
        arcis_validation::check_null_safety(&program, &env)
            .into_iter()
            .map(from_null_safety),
    );

    out
}

fn from_validation(issue: ValidationIssue) -> Diagnostic {
    let (message, severity, line, col, len) = match &issue {
        ValidationIssue::Unused(u) => (
            format!("unused {} `{}`", u.kind.label(), u.name),
            DiagnosticSeverity::WARNING,
            u.line,
            u.col,
            u.name.chars().count(),
        ),
        ValidationIssue::Duplicate(d) => (
            format!(
                "duplicate {} `{}` (first declared at {}:{})",
                d.kind.label(),
                d.name,
                d.first_line,
                d.first_col
            ),
            DiagnosticSeverity::ERROR,
            d.second_line,
            d.second_col,
            d.name.chars().count(),
        ),
        ValidationIssue::BreakOutsideLoop { keyword, line, col } => (
            format!("`{}` outside of a loop", keyword),
            DiagnosticSeverity::ERROR,
            *line,
            *col,
            keyword.chars().count(),
        ),
    };
    let line = line.saturating_sub(1) as u32;
    let character = col.saturating_sub(1) as u32;
    Diagnostic {
        range: Range {
            start: Position { line, character },
            end: Position {
                line,
                character: character.saturating_add(len.max(1) as u32),
            },
        },
        severity: Some(severity),
        source: Some("arcis-lsp".into()),
        message,
        ..Default::default()
    }
}

fn from_null_safety(issue: NullSafetyIssue) -> Diagnostic {
    // Many null-safety errors anchor on an expression deep inside a
    // statement; `Expr` carries no span in this AST, so `line`/`col` fall
    // back to `(0, 0)` (top of file) when nothing more precise is known —
    // still surfaces the issue with its self-descriptive message, just
    // without a tight underline.
    let line = issue.line.saturating_sub(1) as u32;
    let character = issue.col.saturating_sub(1) as u32;
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
        message: issue.message,
        ..Default::default()
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
