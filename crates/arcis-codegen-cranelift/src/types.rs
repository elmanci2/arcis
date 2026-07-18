//! Mapping between Arcis types and Cranelift types, plus shared lowering
//! helpers used by `module.rs`, `function.rs`, `stmt.rs`, `expr.rs`,
//! `builtin.rs`.
//!
//! The Cranelift backend deliberately uses a **flat** representation: every
//! Arcis aggregate (string, vector, object) crosses the IR as a single
//! machine word (`I64`). This makes the call ABI trivial — there is no
//! struct-by-value layout to worry about per target — and keeps the runtime
//! in `arcis_runtime.c` portable plain C. The trade-off is that
//! `ArcisString`, `ArcisVec`, `ArcisObject` are always passed by handle.

use cranelift_codegen::ir::types::{F64, I32, I64, I8};
use cranelift_codegen::ir::Type;

/// Arcis surface types as understood by the lowering. Used for both
/// inferring the type of an expression and for declaring Cranelift
/// variables in the right SSA type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArcisType {
    Number,
    Boolean,
    String,
    Void,
}

impl ArcisType {
    /// Lower this Arcis type to its Cranelift representation.
    pub(crate) fn to_cl(self) -> Type {
        match self {
            ArcisType::Number => F64,
            // Booleans travel as `i8` (0 or 1) — Cranelift doesn't have a
            // first-class boolean type, but `i8` keeps the ABI portable.
            ArcisType::Boolean => I8,
            // Strings are passed as opaque `i64` handles (pointers to the
            // runtime's heap-allocated `{char*, u64, u64}` records).
            ArcisType::String => I64,
            ArcisType::Void => I8, // placeholder; not used for storage
        }
    }

    /// Heuristic: is this type passed as an `I64` handle? (Today: only
    /// `String` is.)
    pub(crate) fn is_handle(self) -> bool {
        matches!(self, ArcisType::String)
    }
}

/// Convert an Arcis type AST node into an [`ArcisType`] when possible.
/// Unknown / non-primitive types (e.g. inline `T[]`, object types) are
/// rejected — they are part of the Phase 2+ scope.
pub(crate) fn from_ast(name: &str, is_array: bool) -> Result<ArcisType, String> {
    if is_array {
        return Err(format!(
            "the Cranelift backend does not yet support arrays of {} (Phase 2)",
            name
        ));
    }
    match name {
        "number" => Ok(ArcisType::Number),
        "boolean" => Ok(ArcisType::Boolean),
        "string" => Ok(ArcisType::String),
        "void" => Ok(ArcisType::Void),
        other => Err(format!(
            "unsupported type `{}` for the Cranelift backend (Phase 2+)",
            other
        )),
    }
}

/// Useful alias for the signature of runtime functions that take two
/// `ArcisString` handles and return `i32` (a 1/0 equality flag, e.g. for
/// `arcis_string_eq`).
pub(crate) fn cl_handle_eq_sig() -> Vec<Type> {
    vec![I64, I64]
}
pub(crate) fn cl_handle_eq_returns() -> Vec<Type> {
    vec![I32]
}
