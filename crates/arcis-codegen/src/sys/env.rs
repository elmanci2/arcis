//! `sys.env.*` — environment variables.
//!
//! Reads, writes and inspects process environment variables via
//! `std::env::var` / `set_var` / `remove_var` / `vars`. Distinct from
//! [`super::proc_env`] which handles *process* state (currentDir,
//! tempDir, etc.).
//!
//! ## Supported operations
//!
//! | Builtin              | Arcis → Rust                                                                | Arcis return |
//! |----------------------|----------------------------------------------------------------------------|--------------|
//! | `sys.env.get(name)`  | `std::env::var(&name).unwrap_or_default()`                                 | `string`     |
//! | `sys.env.set(n, v)`  | `std::env::set_var(&n, &v);`                                               | `void`       |
//! | `sys.env.delete(n)`  | `std::env::remove_var(&n);`                                                 | `void`       |
//! | `sys.env.all()`      | `std::env::vars().map(\|(k, v)\| format!("{}={}", k, v)).collect::<Vec<String>>()` | `string[]` |
//!
//! ## Notes
//!
//! - `get` returns the empty string if the variable is not set.
//! - Setting `PATH`, `HOME`, etc. mutates the child process and any
//!   subsequent subprocesses spawned via `sys.process` / `sys.exec` /
//!   `sys.spawn` inherit it.

use arcis_ast::Expr;

use crate::context::Ctx;

use super::emit_arg_ref;

/// Try to emit `sys.env.<method>(args)`. Returns `true` if this module
/// handled the method.
pub(crate) fn try_emit_method(
    out: &mut String,
    method: &str,
    args: &[Expr],
    ctx: &Ctx,
) -> bool {
    match method {
        "get" => {
            out.push_str("std::env::var(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(").unwrap_or_default()");
            true
        }
        "set" => {
            out.push_str("std::env::set_var(");
            emit_arg_ref(out, args.first(), ctx);
            out.push_str(", ");
            emit_arg_ref(out, args.get(1), ctx);
            out.push(')');
            true
        }
        "delete" => {
            out.push_str("std::env::remove_var(");
            emit_arg_ref(out, args.first(), ctx);
            out.push(')');
            true
        }
        "all" => {
            out.push_str(
                "std::env::vars()\
                 .map(|(k, v)| format!(\"{}={}\", k, v))\
                 .collect::<Vec<String>>()",
            );
            true
        }
        _ => false,
    }
}
