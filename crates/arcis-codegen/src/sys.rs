//! `sys.*` builtins.
//!
//! Translate directly to `std::fs::*` and `std::env::*` without requiring
//! an `import`. The supported operations are:
//!
//! - `readFile(path)` → `std::fs::read_to_string(&path).unwrap()`
//! - `writeFile(path, content)` → `std::fs::write(&path, &content).unwrap()`
//! - `exists(path)` → `std::path::Path::new(&path).exists()`
//! - `deleteFile(path)` → `std::fs::remove_file(&path).unwrap()`
//! - `mkdir(path)` → `std::fs::create_dir(&path).unwrap()`
//! - `listDir(path)` → `std::fs::read_dir(&path).unwrap().map(...).collect()`
//!
//! Unknown `sys.X(...)` calls are emitted as-is (rustc will report).

use arcis_ast::Expr;

use crate::context::Ctx;

/// Emit `sys.<property>(args)` to the output buffer.
pub(crate) fn emit(out: &mut String, property: &str, args: &[Expr], ctx: &Ctx) {
    match property {
        "readFile" => {
            out.push_str("std::fs::read_to_string(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
        }
        "writeFile" => {
            out.push_str("std::fs::write(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(", ");
            emit_arg_ref(out, args.get(1), ctx);
            out.push_str(").unwrap()");
        }
        "exists" => {
            out.push_str("std::path::Path::new(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").exists()");
        }
        "deleteFile" => {
            out.push_str("std::fs::remove_file(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
        }
        "mkdir" => {
            out.push_str("std::fs::create_dir(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
        }
        "listDir" => {
            out.push_str("std::fs::read_dir(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(
                ").unwrap().map(|__arcis_e| __arcis_e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<String>>()",
            );
        }
        _ => {
            // Unknown `sys.X`: emit as-is (rustc will report).
            out.push_str("sys.");
            out.push_str(property);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                crate::expr::emit(out, a, ctx);
            }
            out.push(')');
        }
    }
}

/// Emit `&<expr>` for the argument to a `sys.*` call. `read_to_string` and
/// `Path::new` accept `&S` where `S: AsRef<Path>`, so passing a `String`
/// with `&` forces the right coercion.
fn emit_arg_ref(out: &mut String, arg: Option<&Expr>, ctx: &Ctx) {
    if let Some(a) = arg {
        out.push('&');
        crate::expr::emit(out, a, ctx);
    } else {
        out.push_str("&String::new()");
    }
}