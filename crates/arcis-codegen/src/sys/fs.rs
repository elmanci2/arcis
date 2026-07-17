//! `sys.*` file-system builtins.
//!
//! Translates directly to `std::fs::*` without requiring an `import`.
//! All path arguments are passed by `&` so they satisfy the
//! `AsRef<Path>` bound.
//!
//! ## Supported operations
//!
//! | Builtin                | Arcis → Rust                                                        | Arcis return |
//! |------------------------|--------------------------------------------------------------------|--------------|
//! | `readFile(p)`          | `std::fs::read_to_string(&p).unwrap()`                             | `string`     |
//! | `writeFile(p, c)`      | `std::fs::write(&p, &c).unwrap()`                                  | `void`       |
//! | `readBytes(p)`         | `std::fs::read(&p).unwrap()`                                       | `bytes` (Vec<u8>) |
//! | `writeBytes(p, b)`     | `std::fs::write(&p, &b).unwrap()`                                  | `void`       |
//! | `appendFile(p, t)`     | `OpenOptions::new().append(true).create(true).open(&p).unwrap().write_all(t.as_bytes()).unwrap()` | `void` |
//! | `createFile(p)`        | `std::fs::File::create(&p).unwrap()`                               | `void`       |
//! | `deleteFile(p)`        | `std::fs::remove_file(&p).unwrap()`                                | `void`       |
//! | `deleteDir(p)`         | `std::fs::remove_dir(&p).unwrap()`                                 | `void`       |
//! | `deleteDirAll(p)`      | `std::fs::remove_dir_all(&p).unwrap()`                             | `void`       |
//! | `mkdir(p)`             | `std::fs::create_dir(&p).unwrap()`                                 | `void`       |
//! | `listDir(p)`           | `std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<String>>()` | `string[]` |
//! | `copy(src, dst)`       | `std::fs::copy(&s, &d).unwrap()`                                   | `void`       |
//! | `move(src, dst)`       | `std::fs::rename(&s, &d).unwrap()`                                 | `void`       |
//! | `rename(old, new)`     | `std::fs::rename(&o, &n).unwrap()` (alias of `move`)               | `void`       |
//!
//! ## Notes
//!
//! - `readBytes` / `writeBytes` round-trip a `Vec<u8>` value through the
//!   filesystem. Arcis does not yet expose a `bytes` literal syntax, so
//!   the typical use is `let b = sys.readBytes(p); sys.writeBytes(q, b);`.
//!   Once a `bytes` primitive lands, these builtins will become directly
//!   assignable to a typed `let`.
//! - `move` is matched as the literal property string — Rust's `move`
//!   keyword does not affect a `match` over `&str`, so no `r#` escape is
//!   needed in the codegen.

use arcis_ast::Expr;

use crate::context::Ctx;

use super::emit_arg_ref;

/// Try to emit `sys.<property>(args)`. Returns `true` if this module
/// handled the property.
pub(crate) fn try_emit(
    out: &mut String,
    property: &str,
    args: &[Expr],
    ctx: &Ctx,
) -> bool {
    match property {
        "readFile" => {
            out.push_str("std::fs::read_to_string(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "writeFile" => {
            out.push_str("std::fs::write(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(", ");
            emit_arg_ref(out, args.get(1), ctx);
            out.push_str(").unwrap()");
            true
        }
        "readBytes" => {
            out.push_str("std::fs::read(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "writeBytes" => {
            out.push_str("std::fs::write(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(", &");
            if let Some(b) = args.get(1) {
                crate::expr::emit(out, b, ctx);
            } else {
                out.push_str("Vec::<u8>::new()");
            }
            out.push_str(").unwrap()");
            true
        }
        "appendFile" => {
            // Bind the opened file into a local so we can take `&mut`
            // and call `write_all` via the qualified trait path (no
            // need to import `std::io::Write` at the module top).
            out.push_str(
                "({ let mut __f = std::fs::OpenOptions::new().append(true).create(true).open(",
            );
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(
                ").unwrap(); std::io::Write::write_all(&mut __f, ",
            );
            if let Some(t) = args.get(1) {
                crate::expr::emit(out, t, ctx);
                out.push_str(".as_bytes()");
            } else {
                out.push_str("b\"\"");
            }
            out.push_str(").unwrap(); })");
            true
        }
        "createFile" => {
            out.push_str("std::fs::File::create(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "deleteFile" => {
            out.push_str("std::fs::remove_file(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "deleteDir" => {
            out.push_str("std::fs::remove_dir(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "deleteDirAll" => {
            out.push_str("std::fs::remove_dir_all(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "mkdir" => {
            out.push_str("std::fs::create_dir(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "listDir" => {
            out.push_str("std::fs::read_dir(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(
                ").unwrap().map(|__arcis_e| __arcis_e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<String>>()",
            );
            true
        }
        "copy" => {
            out.push_str("std::fs::copy(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(", ");
            emit_arg_ref(out, args.get(1), ctx);
            out.push_str(").unwrap()");
            true
        }
        "move" | "rename" => {
            out.push_str("std::fs::rename(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(", ");
            emit_arg_ref(out, args.get(1), ctx);
            out.push_str(").unwrap()");
            true
        }
        _ => false,
    }
}
