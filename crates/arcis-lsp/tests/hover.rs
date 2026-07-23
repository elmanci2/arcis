//! Tests for the hover provider.
//!
//! Exercises the different positions a hover can be triggered at:
//! a single keyword, a top-level `sys.X` builtin, a namespace method,
//! and an empty spot.

use arcis_lsp::hover::hover_at;

fn markdown(h: async_lsp::lsp_types::Hover) -> String {
    use async_lsp::lsp_types::HoverContents;
    if let HoverContents::Markup(m) = h.contents {
        m.value
    } else {
        panic!("hover returned non-markup contents")
    }
}

#[test]
fn hover_on_keyword_returns_signature_and_doc() {
    // Cursor sits right after `let`, with the rest of the line trailing
    // in the "after" segment. The identifier under the cursor is `let`.
    let h = hover_at("let x = 0;", "let", " x = 0;").expect("hover should be present for `let`");
    let body = markdown(h);
    assert!(body.contains("let"), "body should mention `let`: {body}");
    assert!(
        body.contains("declare a mutable binding"),
        "body should have the detail line: {body}"
    );
}

#[test]
fn hover_on_top_level_sys_read_file() {
    // Cursor sits after `sys.read` but before continuing into `File(...)`.
    let h = hover_at("sys.readFile(p)", "sys.read", "File(p)")
        .expect("hover should be present for `sys.readFile`");
    let body = markdown(h);
    assert!(body.contains("sys.readFile"));
    assert!(body.contains("(path: string) -> string"));
    assert!(body.contains("reads `p` to a UTF-8"));
}

#[test]
fn hover_on_namespace_method() {
    let h = hover_at("sys.env.get(name)", "sys.env.", "get(name)")
        .expect("hover should be present for env.get");
    let body = markdown(h);
    assert!(body.contains("sys.env.get"));
    assert!(body.contains("(name: string) -> string"));
}

#[test]
fn hover_in_whitespace_returns_none_or_label() {
    // Cursor is between two spaces — no identifier under the cursor.
    let h = hover_at("let x =  ;", "let x =  ", ";");
    // It's OK if either Some or None — but if Some, the label must be meaningful.
    if let Some(h) = h {
        let body = markdown(h);
        assert!(body.len() > 0);
    }
}

#[test]
fn hover_on_inferred_let_shows_type() {
    // `x` has no annotation — hover must show the inferred `number`.
    let text = "let x = 42;\nprint(x);\n";
    let h = hover_at(text, "let ", "x = 42;").expect("hover for user symbol");
    let body = markdown(h);
    assert!(body.contains("number"), "inferred type shown: {body}");
}

#[test]
fn hover_on_inferred_function_return() {
    let text = "function dbl(n: number) {\n    return n * 2;\n}\n";
    let h = hover_at(text, "function db", "l(n: number) {").expect("hover for function");
    let body = markdown(h);
    assert!(body.contains("number"), "inferred return type shown: {body}");
}
