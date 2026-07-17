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

// ── process module ───────────────────────────────────────────────────────

#[test]
fn sys_process_emits_arcis_process_struct_literal() {
    let out = emit(r#"let p = sys.process("git", ["status"]); print(p.stdout);"#);
    // Struct definition is emitted in the root module.
    assert!(
        out.contains("pub struct ArcisProcess"),
        "missing ArcisProcess struct: {out}"
    );
    assert!(
        out.contains("pub stdout: String"),
        "missing stdout field: {out}"
    );
    assert!(
        out.contains("pub exitCode: f64"),
        "missing exitCode field: {out}"
    );
    // Literal construction.
    assert!(out.contains("ArcisProcess {"), "{out}");
    assert!(out.contains("String::from_utf8_lossy"), "{out}");
    assert!(out.contains("status.code()"), "{out}");
}

#[test]
fn sys_process_handles_missing_args() {
    // `sys.process("true")` with no args array should default to `Vec::<String>::new()`.
    let out = emit(r#"sys.process("true");"#);
    assert!(out.contains("Vec::<String>::new()"), "{out}");
    assert!(out.contains("ArcisProcess {"), "{out}");
}

#[test]
fn sys_exec_returns_stdout_as_string() {
    let out = emit(r#"let s: string = sys.exec("echo", ["hi"]);"#);
    assert!(out.contains("std::process::Command::new"), "{out}");
    assert!(out.contains(".output().unwrap().stdout"), "{out}");
    assert!(out.contains("String::from_utf8_lossy"), "{out}");
    assert!(out.contains(".into_owned()"), "{out}");
}

#[test]
fn sys_spawn_returns_pid_as_number() {
    let out = emit(r#"let pid: number = sys.spawn("sleep", ["1"]);"#);
    assert!(out.contains("std::process::Command::new"), "{out}");
    assert!(out.contains(".spawn().unwrap().id()"), "{out}");
    assert!(out.contains("as f64"), "{out}");
}

#[test]
fn sys_kill_uses_kill_binary() {
    let out = emit(r#"sys.kill(123);"#);
    assert!(
        out.contains("Command::new(\"kill\")"),
        "missing kill invocation: {out}"
    );
    assert!(out.contains(".arg(format!"), "{out}");
    assert!(out.contains(".status().unwrap()"), "{out}");
}

#[test]
fn sys_current_pid() {
    let out = emit(r#"let p: number = sys.currentPid();"#);
    assert!(out.contains("std::process::id()"), "{out}");
    assert!(out.contains("as f64"), "{out}");
}

#[test]
fn sys_parent_pid_resolves_at_runtime() {
    let out = emit(r#"let p: number = sys.parentPid();"#);
    assert!(out.contains("cfg!(target_os = \"windows\")"), "{out}");
    assert!(
        out.contains("std::os::unix::process::parent_id"),
        "missing unix parent_id call: {out}"
    );
    assert!(out.contains("as f64"), "{out}");
}

#[test]
fn sys_processes_parses_ps_output() {
    let out = emit(r#"let procs: string[] = sys.processes();"#);
    assert!(out.contains("cfg!(target_os = \"windows\")"), "{out}");
    assert!(out.contains("Command::new(\"ps\")"), "{out}");
    assert!(out.contains("pid=,comm="), "{out}");
    assert!(out.contains("format!(\"pid={};name={}\""), "{out}");
    assert!(out.contains("collect::<Vec<String>>()"), "{out}");
}

// ── sys.env sub-namespace (env vars) ────────────────────────────────────

#[test]
fn sys_env_get() {
    let out = emit(r#"let h: string = sys.env.get("HOME");"#);
    assert!(out.contains("std::env::var"), "{out}");
    assert!(out.contains(".unwrap_or_default()"), "{out}");
}

#[test]
fn sys_env_set() {
    let out = emit(r#"sys.env.set("X", "y");"#);
    assert!(out.contains("std::env::set_var"), "{out}");
    // Trailing `;` is added by the statement emitter, not by us.
    assert!(!out.contains("set_var(...);"), "{out}");
}

#[test]
fn sys_env_delete() {
    let out = emit(r#"sys.env.delete("X");"#);
    assert!(out.contains("std::env::remove_var"), "{out}");
}

#[test]
fn sys_env_all() {
    let out = emit(r#"let vs: string[] = sys.env.all();"#);
    assert!(out.contains("std::env::vars()"), "{out}");
    assert!(out.contains("format!(\"{}={}\""), "{out}");
    assert!(out.contains("collect::<Vec<String>>()"), "{out}");
}

// ── sys.os sub-namespace ────────────────────────────────────────────────

#[test]
fn sys_os_name_runtime_cfg() {
    let out = emit(r#"let n: string = sys.os.name();"#);
    assert!(out.contains("cfg!(target_os = \"linux\")"), "{out}");
    assert!(out.contains("cfg!(target_os = \"macos\")"), "{out}");
    assert!(out.contains("cfg!(target_os = \"windows\")"), "{out}");
    assert!(out.contains("\"linux\""), "{out}");
    assert!(out.contains("\"macos\""), "{out}");
    assert!(out.contains("\"windows\""), "{out}");
}

#[test]
fn sys_os_arch_uses_consts() {
    let out = emit(r#"let a: string = sys.os.arch();"#);
    assert!(out.contains("std::env::consts::ARCH"), "{out}");
    assert!(out.contains(".to_string()"), "{out}");
}

#[test]
fn sys_os_cpu_count() {
    let out = emit(r#"let c: number = sys.os.cpuCount();"#);
    assert!(out.contains("available_parallelism"), "{out}");
    assert!(out.contains("as f64"), "{out}");
}

#[test]
fn sys_os_username_runtime_cfg() {
    let out = emit(r#"let u: string = sys.os.username();"#);
    assert!(out.contains("cfg!(target_os = \"windows\")"), "{out}");
    assert!(out.contains("\"USERNAME\""), "{out}");
    assert!(out.contains("\"USER\""), "{out}");
}

#[test]
fn sys_os_uptime_parses_proc() {
    let out = emit(r#"let u: number = sys.os.uptime();"#);
    assert!(out.contains("cfg!(target_os = \"linux\")"), "{out}");
    assert!(out.contains("/proc/uptime"), "{out}");
}

#[test]
fn sys_os_hostname_subprocess() {
    let out = emit(r#"let h: string = sys.os.hostname();"#);
    assert!(out.contains("Command::new(\"hostname\")"), "{out}");
}

// ── sys.memory sub-namespace ────────────────────────────────────────────

#[test]
fn sys_memory_total_linux_gated() {
    let out = emit(r#"let b: number = sys.memory.total();"#);
    assert!(out.contains("cfg!(target_os = \"linux\")"), "{out}");
    assert!(out.contains("/proc/meminfo"), "{out}");
    assert!(out.contains("MemTotal:"), "{out}");
    assert!(out.contains("as f64 * 1024.0"), "{out}");
}

#[test]
fn sys_memory_free_linux_gated() {
    let out = emit(r#"let b: number = sys.memory.free();"#);
    assert!(out.contains("/proc/meminfo"), "{out}");
    assert!(out.contains("MemFree:"), "{out}");
}

#[test]
fn sys_memory_used_is_total_minus_free() {
    let out = emit(r#"let b: number = sys.memory.used();"#);
    assert!(out.contains("MemTotal:"), "{out}");
    assert!(out.contains("MemFree:"), "{out}");
    assert!(out.contains(") - ("), "{out}");
}

#[test]
fn sys_memory_available_linux_gated() {
    let out = emit(r#"let b: number = sys.memory.available();"#);
    assert!(out.contains("MemAvailable:"), "{out}");
}

// ── sys.cpu sub-namespace ───────────────────────────────────────────────

#[test]
fn sys_cpu_model_linux_gated() {
    let out = emit(r#"let m: string = sys.cpu.model();"#);
    assert!(out.contains("/proc/cpuinfo"), "{out}");
    assert!(out.contains("model name"), "{out}");
    assert!(out.contains("cfg!(target_os = \"linux\")"), "{out}");
}

#[test]
fn sys_cpu_brand_linux_gated() {
    let out = emit(r#"let v: string = sys.cpu.brand();"#);
    assert!(out.contains("vendor_id"), "{out}");
}

#[test]
fn sys_cpu_frequency_linux_gated() {
    let out = emit(r#"let m: number = sys.cpu.frequency();"#);
    assert!(out.contains("cpu MHz"), "{out}");
}

#[test]
fn sys_cpu_usage_two_samples_with_sleep() {
    let out = emit(r#"let p: number = sys.cpu.usage();"#);
    assert!(out.contains("/proc/stat"), "{out}");
    assert!(out.contains("thread::sleep"), "{out}");
    assert!(out.contains("from_millis(100)"), "{out}");
}

#[test]
fn sys_cpu_cores_uses_parallelism() {
    let out = emit(r#"let c: number = sys.cpu.cores();"#);
    assert!(out.contains("available_parallelism"), "{out}");
}

// ── sys.gpu sub-namespace ───────────────────────────────────────────────

#[test]
fn sys_gpu_list_lspci() {
    let out = emit(r#"let g: string[] = sys.gpu.list();"#);
    assert!(out.contains("cfg!(target_os = \"linux\")"), "{out}");
    assert!(out.contains("Command::new(\"lspci\")"), "{out}");
    assert!(out.contains("\"-vmm\""), "{out}");
    assert!(out.contains("vga"), "{out}");
    assert!(out.contains("3d"), "{out}");
    assert!(out.contains("display"), "{out}");
    assert!(out.contains("format!(\"vendor={};name={}\""), "{out}");
}

#[test]
fn sys_gpu_name_first_lspci_entry() {
    let out = emit(r#"let n: string = sys.gpu.name();"#);
    assert!(out.contains("Command::new(\"lspci\")"), "{out}");
    assert!(out.contains(";name="), "{out}");
}

#[test]
fn sys_gpu_vendor_first_lspci_entry() {
    let out = emit(r#"let v: string = sys.gpu.vendor();"#);
    assert!(out.contains("vendor="), "{out}");
}

#[test]
fn sys_gpu_memory_nvidia_smi() {
    let out = emit(r#"let b: number = sys.gpu.memory();"#);
    assert!(out.contains("nvidia-smi"), "{out}");
    assert!(out.contains("memory.total"), "{out}");
    assert!(out.contains("1024.0 * 1024.0"), "{out}");
}

// ── sys.disk sub-namespace ──────────────────────────────────────────────

#[test]
fn sys_disk_list_df() {
    let out = emit(r#"let ds: string[] = sys.disk.list();"#);
    assert!(out.contains("cfg!(target_os = \"windows\")"), "{out}");
    assert!(out.contains("Command::new(\"df\")"), "{out}");
    assert!(out.contains("\"-B1\""), "{out}");
    assert!(out.contains("\"-P\""), "{out}");
    assert!(out.contains("format!(\"mount={};size={};used={};avail={}\""), "{out}");
}

#[test]
fn sys_disk_free_returns_avail_column() {
    let out = emit(r#"let b: number = sys.disk.free("/");"#);
    assert!(out.contains("Command::new(\"df\")"), "{out}");
    // avail is the 3rd column in our tuple (index 2).
    assert!(out.contains("[r.0, r.1, r.2][2]"), "{out}");
}

#[test]
fn sys_disk_used_returns_used_column() {
    let out = emit(r#"let b: number = sys.disk.used("/");"#);
    assert!(out.contains("[r.0, r.1, r.2][1]"), "{out}");
}

#[test]
fn sys_disk_total_returns_size_column() {
    let out = emit(r#"let b: number = sys.disk.total("/");"#);
    assert!(out.contains("[r.0, r.1, r.2][0]"), "{out}");
}

// ── unknown sub-namespace fallback ──────────────────────────────────────

#[test]
fn unknown_sys_subns_call_is_re_emitted_for_rustc() {
    let out = emit(r#"sys.weird.method("x");"#);
    // Fallback should emit `sys.weird.method(...)` verbatim so rustc
    // produces the diagnostic.
    assert!(out.contains("sys.weird.method"), "{out}");
    assert!(out.contains("\"x\""), "{out}");
}

#[test]
fn unknown_sys_subns_method_is_re_emitted() {
    let out = emit(r#"sys.os.notARealMethod();"#);
    assert!(out.contains("sys.os.notARealMethod"), "{out}");
}
