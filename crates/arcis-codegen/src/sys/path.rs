//! `sys.*` path builtins: queries and transformations about paths.
//!
//! Path operations are read-only or path-rewriting; they do not touch the
//! filesystem payload the way [`super::fs`] does.
//!
//! ## Supported operations
//!
//! | Builtin                  | Arcis → Rust                                                       | Arcis return |
//! |--------------------------|-------------------------------------------------------------------|--------------|
//! | `exists(p)`              | `std::path::Path::new(&p).exists()`                               | `boolean`    |
//! | `isFile(p)`              | `std::path::Path::new(&p).is_file()`                              | `boolean`    |
//! | `isDir(p)`               | `std::path::Path::new(&p).is_dir()`                               | `boolean`    |
//! | `fileSize(p)`            | `std::fs::metadata(&p).unwrap().len() as f64`                     | `number`     |
//! | `fileInfo(p)`            | `format!("size={};is_file={};is_dir={};modified_secs={}", ...)`   | `string`     |
//! | `absolute(p)`            | `std::path::Path::new(&p).canonicalize().unwrap_or_else(...).to_string_lossy().into_owned()` | `string`     |
//! | `relative(p)`            | `std::path::Path::new(&p).strip_prefix(cwd).unwrap_or(...).to_string_lossy().into_owned()` | `string`     |
//! | `createSymlink(t, l)`    | `std::fs::symlink(&t, &l).unwrap()`                               | `void`       |
//! | `readLink(p)`            | `std::fs::read_link(&p).unwrap().to_string_lossy().into_owned()`  | `string`     |
//!
//! ## Notes
//!
//! - `fileInfo` returns a summary `string` rather than a structured value
//!   because Arcis does not yet have a way to declare a return type
//!   shape from a builtin. The format is
//!   `size=<bytes>;is_file=<bool>;is_dir=<bool>;modified_secs=<secs>`.
//!   Use `sys.fileSize`, `sys.isFile`, `sys.isDir` for individual values.
//! - `absolute` uses `canonicalize` when possible (resolves `..` and
//!   symlinks) and falls back to the literal path if the file does not
//!   exist.
//! - `relative` strips the current working directory prefix when
//!   possible; if `p` is not under the cwd, the literal path is
//!   returned.
//! - `createSymlink` uses `std::fs::symlink` which is available on
//!   stable Rust 1.78+ on all platforms.

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
        "exists" => {
            out.push_str("std::path::Path::new(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").exists()");
            true
        }
        "isFile" => {
            out.push_str("std::path::Path::new(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").is_file()");
            true
        }
        "isDir" => {
            out.push_str("std::path::Path::new(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").is_dir()");
            true
        }
        "fileSize" => {
            out.push_str("std::fs::metadata(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap().len() as f64");
            true
        }
        "fileInfo" => {
            // All four fields share a single metadata() call via rebinding
            // in a block expression, then format! the summary string.
            out.push_str(
                "(|| -> String { let __m = std::fs::metadata(",
            );
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(
                ").unwrap(); format!(\"size={};is_file={};is_dir={};modified_secs={}\", __m.len(), __m.is_file(), __m.is_dir(), __m.modified().map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)).unwrap_or(0)) })()",
            );
            true
        }
        "absolute" => {
            out.push_str(
                "std::path::Path::new(",
            );
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(
                ").canonicalize().unwrap_or_else(|_| std::path::PathBuf::from(",
            );
            // Re-emit the path argument by-ref for the fallback so
            // `PathBuf::from(&S)` is satisfied.
            if let Some(p) = args.first() {
                emit_arg_ref(out, Some(p), ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(
                ")).to_string_lossy().into_owned()",
            );
            true
        }
        "relative" => {
            out.push_str(
                "std::path::Path::new(",
            );
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(
                ").strip_prefix(std::env::current_dir().unwrap()).unwrap_or(std::path::Path::new(",
            );
            if let Some(p) = args.first() {
                emit_arg_ref(out, Some(p), ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(
                ")).to_string_lossy().into_owned()",
            );
            true
        }
        "createSymlink" => {
            // We canonicalise the target before storing it in the
            // symlink so that `sys.exists(link)` resolves correctly
            // regardless of where the link lives relative to the
            // target. Without this, a relative target like
            // `tmpdir/real.txt` stored in `tmpdir/link.txt` would be
            // resolved relative to `tmpdir/`, yielding a non-existent
            // `tmpdir/tmpdir/real.txt`.
            //
            // `std::os::unix::fs::symlink` (not `std::fs::symlink`) is
            // the canonical unix path; the top-level alias only
            // exists on Windows in recent toolchains.
            out.push_str(
                "std::os::unix::fs::symlink(&std::path::Path::new(",
            );
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(
                ").canonicalize().unwrap_or_else(|_| std::path::PathBuf::from(",
            );
            if let Some(t) = args.first() {
                super::emit_arg_ref(out, Some(t), ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(")), ");
            emit_arg_ref(out, args.get(1), ctx);
            out.push_str(").unwrap()");
            true
        }
        "readLink" => {
            out.push_str("std::fs::read_link(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap().to_string_lossy().into_owned()");
            true
        }
        _ => false,
    }
}
