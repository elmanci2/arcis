//! `sys.*` process-environment builtins.
//!
//! Process-scoped operations: where am I, where can I write, where do I
//! live, etc. These are the **top-level** `sys.X` builtins (e.g. `sys.
//! currentDir()`, `sys.changeDir(...)`).
//!
//! Note that `sys.args` is a *member access* (not a call) and is handled
//! in [`super::emit_member`].
//!
//! Environment-variable accessors live in the sibling
//! [`super::env`] module and are reached via the `sys.env.*` sub-
//! namespace (e.g. `sys.env.get("HOME")`).
//!
//! ## Supported operations
//!
//! | Builtin                  | Arcis → Rust                                                       | Arcis return |
//! |--------------------------|-------------------------------------------------------------------|--------------|
//! | `currentDir()`           | `std::env::current_dir().unwrap().to_string_lossy().into_owned()` | `string`     |
//! | `changeDir(p)`           | `std::env::set_current_dir(&p).unwrap()`                          | `void`       |
//! | `tempDir()`              | `std::env::temp_dir().to_string_lossy().into_owned()`             | `string`     |
//! | `homeDir()`              | `std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default()` | `string` |
//! | `executablePath()`       | `std::env::current_exe().unwrap().to_string_lossy().into_owned()` | `string`     |
//!
//! ## Notes
//!
//! - `homeDir` resolves at runtime, not compile time, so it works on
//!   every target without `#[cfg(...)]` in the generated source.
//! - `changeDir` mutates process state; subsequent `currentDir` calls
//!   in the same program reflect the new working directory.

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
        "currentDir" => {
            out.push_str(
                "std::env::current_dir().unwrap().to_string_lossy().into_owned()",
            );
            true
        }
        "changeDir" => {
            out.push_str("std::env::set_current_dir(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap()");
            true
        }
        "tempDir" => {
            out.push_str(
                "std::env::temp_dir().to_string_lossy().into_owned()",
            );
            true
        }
        "homeDir" => {
            out.push_str(
                "std::env::var(if cfg!(windows) { \"USERPROFILE\" } else { \"HOME\" }).unwrap_or_default()",
            );
            true
        }
        "executablePath" => {
            out.push_str(
                "std::env::current_exe().unwrap().to_string_lossy().into_owned()",
            );
            true
        }
        _ => false,
    }
}
