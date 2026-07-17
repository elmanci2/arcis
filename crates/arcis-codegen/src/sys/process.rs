//! `sys.*` process-management builtins.
//!
//! Wraps `std::process::Command` to spawn child processes and surface
//! their stdout / stderr / exit code. One struct return type is used:
//! `ArcisProcess`, emitted unconditionally by the codegen root module
//! (see `crate::generate_all`).
//!
//! ## Supported operations
//!
//! | Builtin                   | Arcis → Rust                                                                                          | Arcis return |
//! |---------------------------|------------------------------------------------------------------------------------------------------|--------------|
//! | `process(cmd, args)`      | runs `Command::new(&cmd).args(&args).output()`, packs into `ArcisProcess { stdout, stderr, exitCode }` | `ArcisProcess` |
//! | `exec(cmd, args?)`        | `String::from_utf8_lossy(&Command::new(&cmd).args(...).output().unwrap().stdout).into_owned()`        | `string`     |
//! | `spawn(cmd, args?)`       | `Command::new(&cmd).args(...).spawn().unwrap().id() as f64`                                           | `number`     |
//! | `kill(pid)`               | `Command::new("kill").arg(format!("{}", pid)).status().unwrap()` (sends SIGTERM)                     | `void`       |
//! | `currentPid()`            | `std::process::id() as f64`                                                                           | `number`     |
//! | `parentPid()`             | runtime cfg: `unix::process::parent_id` on unix, `0` on windows                                       | `number`     |
//! | `processes()`             | parses `ps -e -o pid=,comm=`, returns `pid=<n>;name=<s>` per line; `[]` on windows                    | `string[]`   |
//!
//! ## Notes
//!
//! - `ArcisProcess` is a fixed struct that codegen always emits at the
//!   root module. The user can write `let p = sys.process(...)` without
//!   a type annotation and rely on Rust inference.
//! - `kill` uses the `kill(1)` binary as a child process for portability;
//!   this avoids depending on `libc`. The exit status is unchecked — a
//!   non-zero exit (e.g. "no such pid") is silently swallowed, matching
//!   the rest of the `sys.*` module.
//! - `processes()` parses `ps` output line by line. Linux and macOS
//!   both ship `ps`. Windows has no equivalent in the standard `PATH`;
//!   we return an empty `Vec` until a Windows backend lands.

use arcis_ast::Expr;

use crate::context::Ctx;

use super::emit_arg_ref;

/// Try to emit `sys.<property>(args)`. Returns `true` if this module
/// handled the property.
pub(crate) fn try_emit_call(
    out: &mut String,
    property: &str,
    args: &[Expr],
    ctx: &Ctx,
) -> bool {
    match property {
        "process" => emit_process(out, args, ctx),
        "exec" => emit_exec(out, args, ctx),
        "spawn" => emit_spawn(out, args, ctx),
        "kill" => emit_kill(out, args, ctx),
        "currentPid" => {
            out.push_str("std::process::id() as f64");
            true
        }
        "parentPid" => {
            // Runtime cfg: unix returns the real parent PID, windows
            // returns 0 (no stable stdlib API for parent PID on windows).
            out.push_str(
                "(if cfg!(target_os = \"windows\") { 0u32 } \
                 else { std::os::unix::process::parent_id() }) as f64",
            );
            true
        }
        "processes" => emit_processes(out),
        _ => false,
    }
}

/// `sys.process(cmd, args)` — run a child, return `ArcisProcess`.
fn emit_process(out: &mut String, args: &[Expr], ctx: &Ctx) -> bool {
    let Some(cmd) = args.first() else {
        return false;
    };
    out.push_str("({ let __o = std::process::Command::new(");
    emit_arg_ref(out, Some(cmd), ctx);
    out.push_str(").args(&");
    if let Some(a) = args.get(1) {
        crate::expr::emit(out, a, ctx);
    } else {
        out.push_str("Vec::<String>::new()");
    }
    out.push_str(
        ").output().unwrap(); ArcisProcess { \
         stdout: String::from_utf8_lossy(&__o.stdout).into_owned(), \
         stderr: String::from_utf8_lossy(&__o.stderr).into_owned(), \
         exitCode: __o.status.code().unwrap_or(-1) as f64 } })",
    );
    true
}

/// `sys.exec(cmd, args?)` — run a child, return stdout as a lossy string.
fn emit_exec(out: &mut String, args: &[Expr], ctx: &Ctx) -> bool {
    let Some(cmd) = args.first() else {
        return false;
    };
    out.push_str(
        "String::from_utf8_lossy(&std::process::Command::new(",
    );
    emit_arg_ref(out, Some(cmd), ctx);
    out.push_str(").args(&");
    if let Some(a) = args.get(1) {
        crate::expr::emit(out, a, ctx);
    } else {
        out.push_str("Vec::<String>::new()");
    }
    out.push_str(
        ").output().unwrap().stdout).into_owned()",
    );
    true
}

/// `sys.spawn(cmd, args?)` — spawn detached, return PID.
fn emit_spawn(out: &mut String, args: &[Expr], ctx: &Ctx) -> bool {
    let Some(cmd) = args.first() else {
        return false;
    };
    out.push_str("std::process::Command::new(");
    emit_arg_ref(out, Some(cmd), ctx);
    out.push_str(").args(&");
    if let Some(a) = args.get(1) {
        crate::expr::emit(out, a, ctx);
    } else {
        out.push_str("Vec::<String>::new()");
    }
    out.push_str(").spawn().unwrap().id() as f64");
    true
}

/// `sys.kill(pid)` — spawn `kill <pid>` as a child (sends SIGTERM).
fn emit_kill(out: &mut String, args: &[Expr], ctx: &Ctx) -> bool {
    let Some(pid) = args.first() else {
        return false;
    };
    out.push_str(
        "std::process::Command::new(\"kill\").arg(format!(\"{}\", ",
    );
    crate::expr::emit(out, pid, ctx);
    out.push_str(
        ")).status().unwrap()",
    );
    true
}

/// `sys.processes()` — list running processes as `pid=<n>;name=<s>` strings.
/// On Windows this returns an empty vector; the parsing path requires `ps`.
fn emit_processes(out: &mut String) -> bool {
    out.push_str(
        "(if cfg!(target_os = \"windows\") { \
         Vec::<String>::new() } else { \
         let __out = String::from_utf8_lossy(&std::process::Command::new(\"ps\")\
         .arg(\"-e\").arg(\"-o\").arg(\"pid=,comm=\")\
         .output().unwrap().stdout).into_owned(); \
         __out.lines().filter(|__l| !__l.trim().is_empty()).map(|__l| { \
         let mut __parts = __l.trim().splitn(2, char::is_whitespace); \
         let __pid = __parts.next().unwrap_or(\"\").to_string(); \
         let __name = __parts.next().unwrap_or(\"\").to_string(); \
         format!(\"pid={};name={}\", __pid, __name) \
         }).collect::<Vec<String>>() })",
    );
    true
}