//! Tests for the diagnostics provider.
//!
//! Each test feeds a `.tsr` snippet to `diagnostics_for` and asserts
//! the right number / severity of diagnostics.

use arcis_lsp::diagnostics::diagnostics_for;

#[test]
fn empty_document_has_no_diagnostics() {
    let diags = diagnostics_for("");
    assert!(
        diags.is_empty(),
        "empty document should produce no diagnostics, got: {diags:#?}"
    );
}

#[test]
fn valid_program_has_no_diagnostics() {
    let diags = diagnostics_for("print(\"hi\");\n");
    assert!(
        diags.is_empty(),
        "valid program should produce no diagnostics, got: {diags:#?}"
    );
}

#[test]
fn syntax_error_is_reported_as_error() {
    let diags = diagnostics_for("let x = ;\n");
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one diagnostic, got: {diags:#?}"
    );
    let d = &diags[0];
    assert_eq!(d.severity, Some(async_lsp::lsp_types::DiagnosticSeverity::ERROR));
    assert!(
        !d.message.is_empty(),
        "diagnostic message should be non-empty"
    );
    assert_eq!(d.source.as_deref(), Some("arcis-lsp"));
}

#[test]
fn unterminated_string_is_reported_as_lex_error() {
    // The lexer should reject the unterminated string with `MissingTerminator`.
    let diags = diagnostics_for("let s = \"unterminated\n");
    assert!(
        !diags.is_empty(),
        "expected at least one diagnostic for unterminated string"
    );
    let first = &diags[0];
    // Lex errors come through as whole-document diagnostics today
    // (the LexError doesn't carry line info yet). Confirm severity.
    assert_eq!(first.severity, Some(async_lsp::lsp_types::DiagnosticSeverity::ERROR));
}

#[test]
fn multi_line_program_parses_clean() {
    let src = "\
let x: number = 1;
let y: number = 2;
let z: number = x + y;
print(z);
";
    let diags = diagnostics_for(src);
    assert!(diags.is_empty(), "got: {diags:#?}");
}