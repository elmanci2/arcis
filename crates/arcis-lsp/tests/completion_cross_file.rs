//! Tests for cross-file completion: namespace-import member completion
//! (`import utils; utils.<TAB>`) and `from x import *;` wildcard imports.
//! These need a real `doc_uri` + a sibling `.tsr` file on disk, since
//! resolution goes through the same file-path logic
//! `definition::goto_definition` uses — unlike `tests/completion.rs`,
//! which only exercises the no-`doc_uri` (single-document) path.

use arcis_lsp::completion::completions_at;
use async_lsp::lsp_types::Url;

fn labels(prefix: &str, full_text: &str, uri: &Url) -> Vec<String> {
    completions_at(prefix, full_text, Some(uri))
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[test]
fn namespace_import_offers_target_modules_exports() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("utils.tsr"),
        "export function add(a: number, b: number): number { return a + b; }\n\
         export const PI: number = 3.14;\n\
         function helper(): void {}\n", // not exported — must NOT appear
    )
    .unwrap();
    let main_path = dir.path().join("main.tsr");
    std::fs::write(&main_path, "import utils;\n").unwrap();
    let uri = Url::from_file_path(&main_path).unwrap();

    let doc = "import utils;\nutils.";
    let labels = labels("utils.", doc, &uri);
    assert!(labels.contains(&"add".to_string()), "missing exported `add`: {labels:?}");
    assert!(labels.contains(&"PI".to_string()), "missing exported `PI`: {labels:?}");
    assert!(!labels.contains(&"helper".to_string()), "non-exported `helper` leaked: {labels:?}");
}

#[test]
fn aliased_namespace_import_still_resolves() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("mate.tsr"),
        "export function sumar(a: number, b: number): number { return a + b; }\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.tsr");
    std::fs::write(&main_path, "import mate as m;\n").unwrap();
    let uri = Url::from_file_path(&main_path).unwrap();

    let doc = "import mate as m;\nm.";
    let labels = labels("m.", doc, &uri);
    assert!(labels.contains(&"sumar".to_string()), "missing exported `sumar` via alias: {labels:?}");
}

#[test]
fn wildcard_import_brings_exports_into_top_level_completion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("consts.tsr"),
        "export const MAX_RETRIES: number = 3;\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.tsr");
    let doc = "from consts import *;\n";
    std::fs::write(&main_path, doc).unwrap();
    let uri = Url::from_file_path(&main_path).unwrap();

    let labels = labels("", doc, &uri);
    assert!(
        labels.contains(&"MAX_RETRIES".to_string()),
        "wildcard-imported `MAX_RETRIES` missing from top-level completion: {labels:?}"
    );
}

#[test]
fn missing_target_file_degrades_to_empty_not_error() {
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().join("main.tsr");
    std::fs::write(&main_path, "import doesnotexist;\n").unwrap();
    let uri = Url::from_file_path(&main_path).unwrap();

    let doc = "import doesnotexist;\ndoesnotexist.";
    // Must not panic; an unresolvable target just yields no completions.
    let labels = labels("doesnotexist.", doc, &uri);
    assert!(labels.is_empty(), "unresolvable module should yield no items, got: {labels:?}");
}
