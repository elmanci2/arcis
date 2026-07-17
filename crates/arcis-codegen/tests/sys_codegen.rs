//! Integration tests for the `sys.*` codegen path.
//!
//! Each test lexes + parses + generates Rust for a small snippet that
//! invokes a single `sys.<name>(...)` builtin, and asserts that the
//! emitted Rust source contains the expected `std::*` call.
//!
//! These tests cover the public surface of `sys` (filesystem, path and
//! environment modules). They do **not** execute the resulting code —
//! they only validate the translation.

use arcis_ast as _;
use arcis_codegen::generate_all;
use arcis_linker::Module;

/// Build a single-module program from a `.tsr` snippet and return the
/// emitted Rust source for that module.
fn emit(src: &str) -> String {
    let tokens = arcis_lexer::lex(src).expect("lex");
    let program = arcis_parser::parse(tokens).expect("parse");
    let module = Module {
        id: "main".into(),
        path: std::path::PathBuf::from("main.tsr"),
        program,
        exports: Default::default(),
    };
    let sources = generate_all(&[module]).expect("codegen");
    let (_, src) = &sources[0];
    src.clone()
}

// ── fs module ────────────────────────────────────────────────────────────

#[test]
fn sys_read_file() {
    let out = emit(r#"sys.readFile("a.txt");"#);
    assert!(out.contains("std::fs::read_to_string"), "{out}");
    assert!(out.contains("\"a.txt\""), "{out}");
}

#[test]
fn sys_write_file() {
    let out = emit(r#"sys.writeFile("a.txt", "hi");"#);
    assert!(out.contains("std::fs::write"), "{out}");
}

#[test]
fn sys_read_bytes() {
    let out = emit(r#"sys.readBytes("a.bin");"#);
    assert!(out.contains("std::fs::read"), "{out}");
    assert!(out.contains("\"a.bin\""), "{out}");
}

#[test]
fn sys_write_bytes() {
    let out = emit(r#"sys.writeBytes("a.bin", bytes);"#);
    assert!(out.contains("std::fs::write"), "{out}");
    assert!(out.contains("&bytes"), "{out}");
}

#[test]
fn sys_append_file() {
    let out = emit(r#"sys.appendFile("log.txt", "msg");"#);
    assert!(out.contains("OpenOptions::new"), "{out}");
    assert!(out.contains(".append(true)"), "{out}");
    assert!(out.contains(".create(true)"), "{out}");
    assert!(out.contains("std::io::Write::write_all"), "{out}");
    assert!(out.contains(".as_bytes()"), "{out}");
}

#[test]
fn sys_create_file() {
    let out = emit(r#"sys.createFile("new.txt");"#);
    assert!(out.contains("std::fs::File::create"), "{out}");
}

#[test]
fn sys_delete_file() {
    let out = emit(r#"sys.deleteFile("a.txt");"#);
    assert!(out.contains("std::fs::remove_file"), "{out}");
}

#[test]
fn sys_delete_dir() {
    let out = emit(r#"sys.deleteDir("empty/");"#);
    assert!(out.contains("std::fs::remove_dir"), "{out}");
    // `remove_dir_all` must not appear here (different builtin).
    assert!(!out.contains("remove_dir_all"), "{out}");
}

#[test]
fn sys_delete_dir_all() {
    let out = emit(r#"sys.deleteDirAll("full/");"#);
    assert!(out.contains("std::fs::remove_dir_all"), "{out}");
}

#[test]
fn sys_mkdir() {
    let out = emit(r#"sys.mkdir("d/");"#);
    assert!(out.contains("std::fs::create_dir"), "{out}");
}

#[test]
fn sys_list_dir() {
    let out = emit(r#"sys.listDir("d/");"#);
    assert!(out.contains("std::fs::read_dir"), "{out}");
    assert!(out.contains("collect::<Vec<String>>()"), "{out}");
}

#[test]
fn sys_copy() {
    let out = emit(r#"sys.copy("src.txt", "dst.txt");"#);
    assert!(out.contains("std::fs::copy"), "{out}");
    assert!(out.contains("\"src.txt\""), "{out}");
    assert!(out.contains("\"dst.txt\""), "{out}");
}

#[test]
fn sys_move() {
    let out = emit(r#"sys.move("src.txt", "dst.txt");"#);
    assert!(out.contains("std::fs::rename"), "{out}");
}

#[test]
fn sys_rename() {
    let out = emit(r#"sys.rename("old.txt", "new.txt");"#);
    assert!(out.contains("std::fs::rename"), "{out}");
}

// ── path module ──────────────────────────────────────────────────────────

#[test]
fn sys_exists() {
    let out = emit(r#"sys.exists("a.txt");"#);
    assert!(out.contains("std::path::Path::new"), "{out}");
    assert!(out.contains(".exists()"), "{out}");
}

#[test]
fn sys_is_file() {
    let out = emit(r#"sys.isFile("a.txt");"#);
    assert!(out.contains(".is_file()"), "{out}");
}

#[test]
fn sys_is_dir() {
    let out = emit(r#"sys.isDir("d/");"#);
    assert!(out.contains(".is_dir()"), "{out}");
}

#[test]
fn sys_file_size() {
    let out = emit(r#"sys.fileSize("a.txt");"#);
    assert!(out.contains("std::fs::metadata"), "{out}");
    assert!(out.contains(".len()"), "{out}");
    assert!(out.contains("as f64"), "{out}");
}

#[test]
fn sys_file_info_emits_format_string() {
    let out = emit(r#"sys.fileInfo("a.txt");"#);
    assert!(out.contains("format!"), "{out}");
    assert!(out.contains("size="), "{out}");
    assert!(out.contains("is_file="), "{out}");
    assert!(out.contains("is_dir="), "{out}");
    assert!(out.contains("modified_secs="), "{out}");
}

#[test]
fn sys_absolute() {
    let out = emit(r#"sys.absolute("a.txt");"#);
    assert!(out.contains("canonicalize"), "{out}");
    assert!(out.contains("to_string_lossy"), "{out}");
}

#[test]
fn sys_relative() {
    let out = emit(r#"sys.relative("a.txt");"#);
    assert!(out.contains("strip_prefix"), "{out}");
    assert!(out.contains("std::env::current_dir"), "{out}");
}

#[test]
fn sys_create_symlink() {
    let out = emit(r#"sys.createSymlink("target", "link");"#);
    assert!(out.contains("std::os::unix::fs::symlink"), "{out}");
    assert!(out.contains("\"target\""), "{out}");
    assert!(out.contains("\"link\""), "{out}");
}

#[test]
fn sys_read_link() {
    let out = emit(r#"sys.readLink("link");"#);
    assert!(out.contains("std::fs::read_link"), "{out}");
}

// ── env module ───────────────────────────────────────────────────────────

#[test]
fn sys_current_dir() {
    let out = emit(r#"sys.currentDir();"#);
    assert!(out.contains("std::env::current_dir"), "{out}");
    assert!(out.contains("to_string_lossy"), "{out}");
}

#[test]
fn sys_change_dir() {
    let out = emit(r#"sys.changeDir("d/");"#);
    assert!(out.contains("std::env::set_current_dir"), "{out}");
}

#[test]
fn sys_temp_dir() {
    let out = emit(r#"sys.tempDir();"#);
    assert!(out.contains("std::env::temp_dir"), "{out}");
}

#[test]
fn sys_home_dir_resolves_at_runtime() {
    let out = emit(r#"sys.homeDir();"#);
    assert!(out.contains("std::env::var"), "{out}");
    // Must use cfg!(...) to pick the env var name at runtime, not a hard-coded literal.
    assert!(out.contains("cfg!(windows)"), "{out}");
    assert!(out.contains("\"USERPROFILE\""), "{out}");
    assert!(out.contains("\"HOME\""), "{out}");
    assert!(out.contains("unwrap_or_default"), "{out}");
}

#[test]
fn sys_executable_path() {
    let out = emit(r#"sys.executablePath();"#);
    assert!(out.contains("std::env::current_exe"), "{out}");
}

// ── member access (sys.args) and fallback ────────────────────────────────

#[test]
fn sys_args_member_access() {
    let out = emit(r#"let a: string[] = sys.args; print("len=" + a.length);"#);
    assert!(out.contains("std::env::args()"), "{out}");
    assert!(out.contains("collect::<Vec<String>>()"), "{out}");
}

#[test]
fn unknown_sys_call_is_re_emitted_for_rustc() {
    let out = emit(r#"sys.notARealCall("x");"#);
    // Fallback should emit `sys.notARealCall(...)` verbatim so rustc
    // produces the diagnostic.
    assert!(out.contains("sys.notARealCall"), "{out}");
    assert!(out.contains("\"x\""), "{out}");
}

#[test]
fn unknown_sys_member_access_is_re_emitted() {
    let out = emit(r#"let x: string = sys.weird;"#);
    assert!(out.contains("sys.weird"), "{out}");
}
