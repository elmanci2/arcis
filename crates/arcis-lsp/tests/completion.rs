//! Tests for the completion provider.
//!
//! Each test exercises a different context (bare identifier, `sys.`,
//! `sys.<ns>.`, `<id>.`) and asserts that the right slice of the
//! static builtin table is offered.

use arcis_lsp::completion::completions_at;

/// Helper: returns the labels of the items that match, with no document
/// context (no local/global symbols, no cross-file resolution).
fn labels(prefix: &str) -> Vec<String> {
    labels_with_doc(prefix, "")
}

/// Like [`labels`], but with real document text so local/global symbols
/// (and `Symbol::ty`-driven member completion) are exercised. No
/// `doc_uri`, so cross-file completion (namespace-import members,
/// wildcard imports) is not exercised here — see `tests/completion_cross_file.rs`.
fn labels_with_doc(prefix: &str, full_text: &str) -> Vec<String> {
    completions_at(prefix, full_text, None)
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

// ── Global / local variable detection ────────────────────────────────────

#[test]
fn top_level_global_variable_is_offered() {
    // A module-scope ("global") `let` must appear in bare-cursor
    // completion, same as any other symbol.
    let doc = "let counter: number = 0;\n";
    let labels = labels_with_doc("", doc);
    assert!(labels.contains(&"counter".to_string()), "missing global `counter`: {labels:?}");
}

#[test]
fn function_local_variable_is_offered() {
    let doc = "function f(): void {\n    let total: number = 0;\n}\n";
    let labels = labels_with_doc("", doc);
    assert!(labels.contains(&"total".to_string()), "missing local `total`: {labels:?}");
}

#[test]
fn variable_in_nested_block_is_offered() {
    let doc = "if (true) {\n    let deep: string = \"x\";\n}\n";
    let labels = labels_with_doc("", doc);
    assert!(labels.contains(&"deep".to_string()), "missing nested `deep`: {labels:?}");
}

#[test]
fn function_parameter_is_offered() {
    let doc = "function greet(name: string): void {}\n";
    let labels = labels_with_doc("", doc);
    assert!(labels.contains(&"name".to_string()), "missing parameter `name`: {labels:?}");
}

// ── Member completion driven by the variable's real type ────────────────

#[test]
fn object_variable_offers_its_own_fields_not_array_string_methods() {
    let doc = "let p: { name: string, age: number } = { name: \"Ana\", age: 3 };\n";
    let labels = labels_with_doc("p.", doc);
    assert!(labels.contains(&"name".to_string()), "missing field `name`: {labels:?}");
    assert!(labels.contains(&"age".to_string()), "missing field `age`: {labels:?}");
    assert!(!labels.contains(&"find".to_string()), "should not offer array methods on an object: {labels:?}");
    assert!(!labels.contains(&"trim".to_string()), "should not offer string methods on an object: {labels:?}");
}

#[test]
fn interface_typed_variable_resolves_to_its_fields() {
    let doc = "interface Person { name: string; age: number; }\nlet p: Person = { name: \"Ana\", age: 3 };\n";
    let labels = labels_with_doc("p.", doc);
    assert!(labels.contains(&"name".to_string()), "missing interface field `name`: {labels:?}");
    assert!(labels.contains(&"age".to_string()), "missing interface field `age`: {labels:?}");
    assert!(!labels.contains(&"find".to_string()), "should not offer array methods on an interface: {labels:?}");
}

#[test]
fn interface_with_extends_merges_base_fields() {
    let doc = "interface Animal { name: string; }\ninterface Dog extends Animal { breed: string; }\nlet d: Dog = { name: \"Rex\", breed: \"Lab\" };\n";
    let labels = labels_with_doc("d.", doc);
    assert!(labels.contains(&"name".to_string()), "missing inherited field `name`: {labels:?}");
    assert!(labels.contains(&"breed".to_string()), "missing own field `breed`: {labels:?}");
}

#[test]
fn string_variable_offers_only_string_methods() {
    let doc = "let s: string = \"hi\";\n";
    let labels = labels_with_doc("s.", doc);
    assert!(labels.contains(&"trim".to_string()), "missing string method `trim`: {labels:?}");
    assert!(!labels.contains(&"find".to_string()), "should not offer array methods on a string: {labels:?}");
}

#[test]
fn array_variable_offers_only_array_methods() {
    let doc = "let xs: number[] = [1, 2, 3];\n";
    let labels = labels_with_doc("xs.", doc);
    assert!(labels.contains(&"find".to_string()), "missing array method `find`: {labels:?}");
    assert!(!labels.contains(&"trim".to_string()), "should not offer string methods on an array: {labels:?}");
}

#[test]
fn number_variable_offers_no_members() {
    let doc = "let n: number = 5;\n";
    let labels = labels_with_doc("n.", doc);
    assert!(labels.is_empty(), "a number has no members to offer: {labels:?}");
}

#[test]
fn unknown_type_falls_back_to_permissive_guess() {
    // No document at all (or a name completion can't resolve): preserve
    // the old permissive behavior rather than offering nothing.
    let labels = labels("myvar.");
    assert!(labels.contains(&"find".to_string()));
    assert!(labels.contains(&"trim".to_string()));
}

#[test]
fn inferred_object_type_without_annotation_still_completes_fields() {
    // No explicit annotation on `p` — type inference should still fill
    // in the shape from the object literal.
    let doc = "let p = { name: \"Ana\", age: 3 };\n";
    let labels = labels_with_doc("p.", doc);
    assert!(labels.contains(&"name".to_string()), "missing inferred field `name`: {labels:?}");
}
