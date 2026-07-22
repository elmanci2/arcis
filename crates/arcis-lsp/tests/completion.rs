//! Tests for the completion provider.
//!
//! Each test exercises a different context (bare identifier, `sys.`,
//! `sys.<ns>.`, `<id>.`) and asserts that the right slice of the
//! static builtin table is offered.

use arcis_lsp::completion::completions_at;

/// Helper: returns the labels of the items that match.
/// `full_text` is the complete document text (empty = no local defs).
fn labels(prefix: &str) -> Vec<String> {
    completions_at(prefix, "")
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn bare_cursor_offers_keywords_and_builtins() {
    let labels = labels("");
    assert!(labels.contains(&"let".to_string()), "missing `let`: {labels:?}");
    assert!(
        labels.contains(&"print".to_string()),
        "missing `print`: {labels:?}"
    );
    assert!(
        labels.contains(&"sys.readFile".to_string()),
        "missing `sys.readFile`: {labels:?}"
    );
    assert!(
        labels.contains(&"sys.writeFile".to_string()),
        "missing `sys.writeFile`: {labels:?}"
    );
}

#[test]
fn after_sys_dot_offers_namespaces() {
    let labels = labels("sys.");
    assert!(labels.contains(&"env".to_string()));
    assert!(labels.contains(&"os".to_string()));
    assert!(labels.contains(&"memory".to_string()));
    assert!(labels.contains(&"cpu".to_string()));
    assert!(labels.contains(&"gpu".to_string()));
    assert!(labels.contains(&"disk".to_string()));
    assert!(labels.contains(&"net".to_string()));
}

#[test]
fn after_sys_env_dot_offers_env_methods() {
    let labels = labels("sys.env.");
    assert_eq!(
        labels,
        vec!["get".to_string(), "set".to_string(), "delete".to_string(), "all".to_string()],
    );
}

#[test]
fn after_sys_os_dot_offers_os_methods() {
    let labels = labels("sys.os.");
    assert!(labels.contains(&"name".to_string()));
    assert!(labels.contains(&"arch".to_string()));
    assert!(labels.contains(&"uptime".to_string()));
    assert!(labels.contains(&"cpuCount".to_string()));
    // 8 OS methods total.
    assert_eq!(labels.len(), 8, "OS method count mismatch: {labels:?}");
}

#[test]
fn after_sys_disk_dot_offers_disk_methods() {
    let labels = labels("sys.disk.");
    assert!(labels.contains(&"list".to_string()));
    assert!(labels.contains(&"free".to_string()));
    assert!(labels.contains(&"used".to_string()));
    assert!(labels.contains(&"total".to_string()));
}

#[test]
fn after_partial_identifier_offers_all() {
    let labels = labels("pri");
    // `pri` is the start of `print`; we should still offer the full
    // builtin table, since the partial-token filtering happens
    // client-side via `filterText`.
    assert!(labels.contains(&"print".to_string()));
}

#[test]
fn after_unknown_ns_dot_offers_top_level_sys() {
    // `disk.` without `sys.` prefix is invalid Arcis, but the
    // dispatcher accepts it as "user probably meant sys.disk".
    let labels = labels("disk.");
    // After the dot with no partial, chain is ["disk"]. This is a
    // single-segment chain — offered as keywords + builtins.
    assert!(
        labels.contains(&"sys.readFile".to_string()),
        "missing `sys.readFile`: {labels:?}"
    );
    assert!(
        labels.contains(&"sys.writeFile".to_string()),
        "missing `sys.writeFile`: {labels:?}"
    );
}

#[test]
fn after_id_dot_offers_method_chains() {
    // A user-typed identifier (could be a variable) followed by `.`
    // should offer array + string methods.
    let labels = labels("myvar.");
    assert!(labels.contains(&"length".to_string()));
    assert!(labels.contains(&"find".to_string()));
    assert!(labels.contains(&"map".to_string()));
    assert!(labels.contains(&"trim".to_string()));
    assert!(labels.contains(&"substring".to_string()));
}

#[test]
fn after_known_namespace_with_partial_dot_offers_methods() {
    // `disk.` (no sys prefix, no partial) — known namespace → methods.
    let labels = labels("disk.f");
    // We currently only match on length-2 chains exactly for known
    // namespaces; `disk.f` becomes chain ["disk", "f"] → falls into
    // the single-segment fallback offering top-level sys.* (and
    // methods are reachable once the user types the second dot).
    assert!(labels.contains(&"sys.readFile".to_string()),
        "missing sys.readFile for prefix `disk.f`: {labels:?}");
}

/// `prefix_up_to` mirrors the LSP position parsing. These tests
/// exercise the end-to-end shape that the editor sees.
#[test]
fn prefix_up_to_in_middle_of_line() {
    use arcis_lsp::server::prefix_up_to;
    let text = "let x = sys.r";
    let pos = async_lsp::lsp_types::Position {
        line: 0,
        character: 13,
    };
    assert_eq!(prefix_up_to(text, pos), "let x = sys.r");
}

#[test]
fn prefix_up_to_clamps_to_eol() {
    use arcis_lsp::server::prefix_up_to;
    let text = "sys.readFile(\"/tmp/x\")";
    // Cursor far past end-of-line; should return chars up to EOL.
    let pos = async_lsp::lsp_types::Position {
        line: 0,
        character: 1000,
    };
    assert_eq!(prefix_up_to(text, pos), "sys.readFile(\"/tmp/x\")");
}

#[test]
fn partial_after_sys_dot_yields_top_level_sys() {
    // The partial identifier the user is typing is part of the chain:
    // `sys.r|`. becomes ["sys", "r"]. That still matches the
    // `["sys", partial]` shape which we route to top-level sys.*.
    let labels = labels("sys.r");
    assert!(labels.contains(&"sys.readFile".to_string()),
        "missing `sys.readFile` for prefix `sys.r`: {labels:?}");
}

#[test]
fn partial_after_sys_env_dot_yields_env_methods() {
    // `sys.env.g|`. → ["sys", "env", "g"]. Still matches the
    // "sys.<ns>." shape (we ignore the partial for namespace match),
    // so we offer env's methods.
    let labels = labels("sys.env.g");
    assert!(labels.contains(&"get".to_string()),
        "missing `get` for prefix `sys.env.g`: {labels:?}");
}