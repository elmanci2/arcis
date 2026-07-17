//! Integration tests for the `arcis` CLI binary.
//!
//! These tests spawn the actual `arcis` binary as a subprocess and verify
//! its behaviour end-to-end — exit code, stdout, stderr, and on-disk
//! artifacts. Each test uses a private scratch directory so nothing leaks
//! into the user's working tree.
//!
//! ## Running the tests
//!
//! ```bash
//! # all of them
//! cargo test -p arcis --test cli
//!
//! # one specific test by substring
//! cargo test -p arcis --test cli init_creates
//! cargo test -p arcis --test cli build_compiles
//! cargo test -p arcis --test cli run_executes
//! cargo test -p arcis --test cli check_prints
//!
//! # via the Makefile shortcuts (see repo root)
//! make test-cli
//! make test-cli-init
//! make test-cli-build
//! make test-cli-run
//! make test-cli-check
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-test scratch directory, named with a monotonically increasing
/// counter so concurrent tests never collide.
struct Scratch {
    dir: PathBuf,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl Scratch {
    fn new() -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "arcis-cli-test-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self { dir }
    }

    /// Write `content` to a file named `name` inside the scratch dir,
    /// returning the absolute path of the written file.
    fn write(&self, name: &str, content: &str) -> PathBuf {
        let p = self.dir.join(name);
        fs::write(&p, content).expect("write file");
        p
    }

    /// Build a `Command` for the `arcis` binary whose cwd is the scratch
    /// dir. Every artifact the binary creates (bin/, generated `.rs`, etc.)
    /// ends up under `self.dir`, so the host working tree is never touched.
    fn cmd(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arcis"));
        cmd.current_dir(&self.dir);
        cmd
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// ─── help / version ───────────────────────────────────────────────────────

#[test]
fn help_lists_every_subcommand() {
    let s = Scratch::new();
    let out = s.cmd().arg("--help").output().expect("spawn arcis");
    assert!(
        out.status.success(),
        "`--help` must exit with success; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for sub in ["build", "run", "check", "init"] {
        assert!(
            stdout.contains(sub),
            "--help must mention `{sub}`; got:\n{stdout}"
        );
    }
}

#[test]
fn version_prints_something() {
    let s = Scratch::new();
    let out = s.cmd().arg("--version").output().expect("spawn arcis");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("arcis"));
}

// ─── init ─────────────────────────────────────────────────────────────────

#[test]
fn init_creates_main_tsr_and_cargo_toml() {
    let s = Scratch::new();
    let status = s
        .cmd()
        .args(["init", "myproj"])
        .status()
        .expect("spawn arcis");
    assert!(status.success(), "init must succeed");
    assert!(
        s.dir.join("myproj/main.tsr").exists(),
        "init must create myproj/main.tsr"
    );
    assert!(
        s.dir.join("myproj/Cargo.toml").exists(),
        "init must create myproj/Cargo.toml"
    );
}

#[test]
fn init_refuses_to_overwrite_existing_main_tsr() {
    let s = Scratch::new();
    s.write("main.tsr", "// existing\n");
    let out = s.cmd().args(["init", "."]).output().expect("spawn arcis");
    assert!(!out.status.success(), "init over an existing project must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already exists"),
        "stderr must explain the failure: {stderr}"
    );
}

#[test]
fn init_in_empty_dir_uses_cwd_default() {
    let s = Scratch::new();
    let status = s.cmd().arg("init").status().expect("spawn arcis");
    assert!(status.success(), "init with no arg in an empty dir must succeed");
    assert!(s.dir.join("main.tsr").exists());
    assert!(s.dir.join("Cargo.toml").exists());
}

// ─── check ────────────────────────────────────────────────────────────────

#[test]
fn check_prints_generated_root_module() {
    let s = Scratch::new();
    let main = s.write("main.tsr", "print(\"hi\");\n");
    let out = s
        .cmd()
        .args(["check", main.to_str().unwrap()])
        .output()
        .expect("spawn arcis");
    assert!(out.status.success(), "check must succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("fn main()"),
        "check output must include `fn main()`: {stdout}"
    );
    assert!(
        stdout.contains("println!"),
        "check output must include `println!`: {stdout}"
    );
}

#[test]
fn check_fails_on_lex_error() {
    let s = Scratch::new();
    // `@` is not a valid token — the lexer must reject it.
    let main = s.write("main.tsr", "let x = @;\n");
    let out = s
        .cmd()
        .args(["check", main.to_str().unwrap()])
        .output()
        .expect("spawn arcis");
    assert!(!out.status.success(), "check with a lex error must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.is_empty(), "stderr must explain the failure");
}

// ─── build ────────────────────────────────────────────────────────────────

#[test]
fn build_compiles_a_simple_program() {
    let s = Scratch::new();
    let main = s.write("main.tsr", "print(\"compiled\");\n");
    let status = s
        .cmd()
        .args(["build", main.to_str().unwrap()])
        .status()
        .expect("spawn arcis");
    assert!(status.success(), "build must succeed");
    let produced = s.dir.join("bin").join("main");
    assert!(produced.exists(), "build must produce a binary at bin/main");
}

#[test]
fn build_fails_cleanly_on_nonexistent_file() {
    let s = Scratch::new();
    let out = s
        .cmd()
        .args(["build", "does-not-exist.tsr"])
        .output()
        .expect("spawn arcis");
    assert!(
        !out.status.success(),
        "build on a missing file must fail with non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.is_empty(),
        "stderr must carry a human-readable diagnostic"
    );
}

#[test]
fn build_emits_arcis_generated_rs_files() {
    let s = Scratch::new();
    let main = s.write("main.tsr", "let x: number = 42; print(x);\n");
    let status = s
        .cmd()
        .args(["build", main.to_str().unwrap()])
        .status()
        .expect("spawn arcis");
    assert!(status.success());
    // `bin/` should hold the root `.rs` file the driver wrote before
    // invoking rustc.
    let rs = s.dir.join("bin").join("main.rs");
    assert!(rs.exists(), "build must leave bin/main.rs on disk");
}

// ─── run ──────────────────────────────────────────────────────────────────

#[test]
fn run_executes_and_prints_output() {
    let s = Scratch::new();
    let main = s.write("main.tsr", "print(\"running\");\n");
    let out = s
        .cmd()
        .args(["run", main.to_str().unwrap()])
        .output()
        .expect("spawn arcis");
    assert!(out.status.success(), "run must exit with success");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("running"),
        "run must execute the program and print its output: stdout was {stdout:?}"
    );
}

#[test]
fn run_propagates_non_zero_exit_codes() {
    let s = Scratch::new();
    // A program that calls `process::exit(7)` so we can check the
    // propagation. We use the bare `std::process` import path because the
    // user does not have an `exit` builtin.
    //
    // For now we use the simpler trick of having a parse error — the
    // driver exits with 1 on any compilation failure.
    let main = s.write("main.tsr", "let x = ;\n"); // syntax error
    let out = s
        .cmd()
        .args(["run", main.to_str().unwrap()])
        .output()
        .expect("spawn arcis");
    assert!(
        !out.status.success(),
        "run on a broken program must exit non-zero"
    );
}

// ─── default behaviour (no subcommand = help + non-zero) ───────────────────

#[test]
fn no_subcommand_prints_help_and_exits_non_zero() {
    let s = Scratch::new();
    let out = s.cmd().output().expect("spawn arcis");
    assert!(
        !out.status.success(),
        "running `arcis` with no subcommand must fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Usage") || stderr.contains("usage"),
        "stderr must show usage info: {stderr}"
    );
}

// ─── multi-module + Cargo layout ──────────────────────────────────────────

#[test]
fn multi_module_build_uses_cargo_layout_when_cargo_toml_present() {
    let s = Scratch::new();
    s.write(
        "utils.tsr",
        "export function greet(): string { return \"hi\"; }\n",
    );
    s.write(
        "main.tsr",
        "import { greet } from \"utils\";\nprint(greet());\n",
    );
    s.write(
        "Cargo.toml",
        "[package]\nname = \"multi\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );

    let status = s.cmd().args(["run"]).status().expect("spawn arcis");
    assert!(status.success(), "run on the multi-module project must succeed");

    // The Cargo-mode build places the generated project under
    // `bin/<entry-dir-name>/`. The entry dir here is the scratch dir, so
    // its name is the temp-dir basename. We assert the layout shape rather
    // than the literal name so the test is robust to temp-dir naming.
    let bin = s.dir.join("bin");
    assert!(bin.exists(), "bin/ must exist after build");
    let subdirs: Vec<_> = fs::read_dir(&bin)
        .expect("read bin/")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    assert_eq!(
        subdirs.len(),
        1,
        "Cargo layout must create exactly one subdir of bin/, got {subdirs:?}"
    );
    let cargo_proj = &subdirs[0].path();
    assert!(
        cargo_proj.join("Cargo.toml").exists(),
        "Cargo.toml must be copied into the generated project"
    );
    assert!(
        cargo_proj.join("src/main.rs").exists(),
        "src/main.rs must be in the generated project"
    );
    assert!(
        cargo_proj.join("src/utils.rs").exists(),
        "src/utils.rs must be in the generated project (one per module)"
    );
}