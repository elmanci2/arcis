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
        "import { greet } from \"utils\";\nprint(greet());\n",
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
        "import { missing } from \"doesnotexist\";\n",
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
        "import { f } from \"b\";\nexport function g() { return 1; }\n",
    )
    .expect("write a.tsr");
    fs::write(
        tmp.path().join("b.tsr"),
        "import { g } from \"a\";\nexport function f() { return 2; }\n",
    )
    .expect("write b.tsr");
    fs::write(
        tmp.path().join("main.tsr"),
        "import { g } from \"a\";\nprint(\"ok\");\n",
    )
    .expect("write main.tsr");

    let result = resolve(tmp.path());
    assert!(result.is_err(), "cycles must be detected");
}

#[test]
fn sanitises_invalid_module_names() {
    let tmp = TempDir::new();
    // Hyphens and leading digits are not valid Rust identifiers; the
    // linker sanitises them so users can name their files however they
    // like. `my-utils` becomes `my_utils` internally.
    fs::write(
        tmp.path().join("my-utils.tsr"),
        "export const X = 1;\n",
    )
    .expect("write my-utils.tsr");
    fs::write(
        tmp.path().join("main.tsr"),
        "import { X } from \"my-utils\";\n",
    )
    .expect("write main.tsr");

    let modules = resolve(tmp.path()).expect("hyphenated names must be sanitised");
    let utils_mod = modules
        .iter()
        .find(|m| m.path.file_name().is_some_and(|f| f == "my-utils.tsr"))
        .expect("utils module loaded");
    assert_eq!(utils_mod.id, "my_utils");
}