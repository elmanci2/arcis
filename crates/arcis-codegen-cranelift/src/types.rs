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
    /// Opaque `I64` handle to an `ArcisVec*` (allocated by the runtime).
    Array,
    /// Opaque `I64` handle to an `ArcisObject*` (allocated by the runtime).
    Object,
}

impl ArcisType {
    /// Lower this Arcis type to its Cranelift representation.
    pub(crate) fn to_cl(self) -> Type {
        match self {
            ArcisType::Number => F64,
            ArcisType::Boolean => I8,
            ArcisType::String => I64,
            ArcisType::Void => I8, // placeholder; not used for storage
            ArcisType::Array => I64,
            ArcisType::Object => I64,
        }
    }

    /// Heuristic: is this type passed as an `I64` handle?
    pub(crate) fn is_handle(self) -> bool {
        matches!(self, ArcisType::String | ArcisType::Array | ArcisType::Object)
    }
}

/// Convert an Arcis type AST node into an [`ArcisType`] when possible.
pub(crate) fn from_ast(name: &str, is_array: bool) -> Result<ArcisType, String> {
    if is_array {
        // `T[]` becomes ArcisType::Array regardless of the inner type.
        // The inner type is stored in the Type AST node but the runtime
        // stores everything as i64 handles anyway.
        return Ok(ArcisType::Array);
    }
    match name {
        "number" => Ok(ArcisType::Number),
        "boolean" => Ok(ArcisType::Boolean),
        "string" => Ok(ArcisType::String),
        "void" => Ok(ArcisType::Void),
        other => {
            // Could be an object type with a __Obj hash name. For now
            // we accept any non-primitive as Object.
            if other.starts_with("__Obj") || !other.is_empty() {
                Ok(ArcisType::Object)
            } else {
                Err(format!(
                    "unsupported type `{}` for the Cranelift backend",
                    other
                ))
            }
        }
    }
}
