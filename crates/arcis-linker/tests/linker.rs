//! Integration tests for the `arcis-linker` crate.
//!
//! These tests exercise the **public** API of the linker by writing tiny
//! `.tsr` fixtures into a temp directory, resolving them, and asserting on
//! the resulting module list.

use arcis_linker::resolve;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-test scratch directory, named with a monotonically increasing
/// counter so concurrent tests never collide.
struct TempDir(PathBuf);

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl TempDir {
    fn new() -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!(
            "arcis-linker-test-{}-{}",
            std::process::id(),
            id
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create temp dir");
        Self(base)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn resolves_single_file_entry() {
    let tmp = TempDir::new();
    fs::write(tmp.path().join("main.tsr"), "print(\"hi\");").expect("write main.tsr");

    let modules = resolve(tmp.path()).expect("linker must succeed");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].id, "main");
}

#[test]
fn resolves_multi_file_program_via_imports() {
    let tmp = TempDir::new();
    fs::write(
        tmp.path().join("utils.tsr"),
        "export function greet(): string { return \"hello\"; }\n",
    )
    .expect("write utils.tsr");
    fs::write(
        tmp.path().join("main.tsr"),
        "from utils import greet;\nprint(greet());\n",
    )
    .expect("write main.tsr");

    let modules = resolve(tmp.path()).expect("linker must succeed");
    assert_eq!(modules.len(), 2);

    // The entry point comes first regardless of file-system order.
    assert_eq!(modules[0].id, "main");
    assert!(modules.iter().any(|m| m.id == "utils"));
}

#[test]
fn flags_missing_import_target() {
    let tmp = TempDir::new();
    fs::write(
        tmp.path().join("main.tsr"),
        "from doesnotexist import missing;\n",
    )
    .expect("write main.tsr");

    let result = resolve(tmp.path());
    assert!(result.is_err(), "missing target must fail");
}

#[test]
fn flags_cyclic_dependencies() {
    let tmp = TempDir::new();
    fs::write(
        tmp.path().join("a.tsr"),
        "from b import f;\nexport function g() { return 1; }\n",
    )
    .expect("write a.tsr");
    fs::write(
        tmp.path().join("b.tsr"),
        "from a import g;\nexport function f() { return 2; }\n",
    )
    .expect("write b.tsr");
    fs::write(
        tmp.path().join("main.tsr"),
        "from a import g;\nprint(\"ok\");\n",
    )
    .expect("write main.tsr");

    let result = resolve(tmp.path());
    assert!(result.is_err(), "cycles must be detected");
}

#[test]
fn resolves_namespace_import() {
    let tmp = TempDir::new();
    fs::write(
        tmp.path().join("utils.tsr"),
        "export const X = 1;\n",
    )
    .expect("write utils.tsr");
    fs::write(
        tmp.path().join("main.tsr"),
        "import utils;\nprint(utils.X);\n",
    )
    .expect("write main.tsr");

    let modules = resolve(tmp.path()).expect("namespace import must succeed");
    assert_eq!(modules.len(), 2);
    assert_eq!(modules[0].id, "main");
    assert!(modules.iter().any(|m| m.id == "utils"));
}

#[test]
fn resolves_from_import_wildcard() {
    let tmp = TempDir::new();
    fs::write(
        tmp.path().join("lib.tsr"),
        "export const A = 1;\nexport const B = 2;\n",
    )
    .expect("write lib.tsr");
    fs::write(
        tmp.path().join("main.tsr"),
        "from lib import *;\nprint(A + B);\n",
    )
    .expect("write main.tsr");

    let modules = resolve(tmp.path()).expect("wildcard import must succeed");
    assert_eq!(modules.len(), 2);
}