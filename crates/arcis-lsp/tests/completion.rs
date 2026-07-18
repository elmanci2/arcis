//! Tests for the completion provider.
//!
//! Each test exercises a different context (bare identifier, `sys.`,
//! `sys.<ns>.`, `<id>.`) and asserts that the right slice of the
//! static builtin table is offered.

use arcis_lsp::completion::completions_at;

/// Helper: returns the labels of the items that match.
fn labels(prefix: &str) -> Vec<String> {
    completions_at(prefix)
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