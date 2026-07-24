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

use cranelift_codegen::ir::types::{F64, I64, I8};
use cranelift_codegen::ir::Type;

use arcis_ast::Type as AstType;

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

    /// Return the Arcis surface name of this type.
    pub(crate) fn name(self) -> &'static str {
        match self {
            ArcisType::Number => "number",
            ArcisType::Boolean => "boolean",
            ArcisType::String => "string",
            ArcisType::Void => "void",
            ArcisType::Array => "array",
            ArcisType::Object => "object",
        }
    }
}

/// Convert an Arcis type AST node into an [`ArcisType`] when possible.
///
/// `enum_names` — every declared `enum`'s name (from
/// `collect::collect_enums`). Enums have no dedicated `ArcisType`; a
/// `Named(name)` matching a known enum resolves to `Number` (their values
/// are plain `f64` constants, see `expr.rs`'s `Expr::Member` handling),
/// instead of falling through to the generic `Object` fallback below.
pub(crate) fn from_ast(ty: &AstType, enum_names: &std::collections::HashSet<String>) -> Result<ArcisType, String> {
    if let AstType::Named(name) = ty {
        if enum_names.contains(name) {
            return Ok(ArcisType::Number);
        }
    }
    match ty {
        // `T?` shares its inner type's machine representation — "missing"
        // is an in-band sentinel (see `expr::null_sentinel`): handle 0 for
        // string/array/object, a canonical NaN payload for number, 2 for
        // boolean. The null-safety checker guarantees optionals are
        // resolved before any typed operation, so the sentinel can never
        // leak into arithmetic/derefs.
        AstType::Optional(inner) => from_ast(inner, enum_names),
        // `T[]` becomes ArcisType::Array regardless of the inner type.
        // The inner type is stored in the Type AST node but the runtime
        // stores everything as i64 handles anyway.
        AstType::Array(_) => Ok(ArcisType::Array),
        AstType::Object { .. } => Ok(ArcisType::Object),
        AstType::Null | AstType::Undefined => Ok(ArcisType::Void),
        AstType::Primitive(name) => match name.as_str() {
            "number" | "bigint" => Ok(ArcisType::Number),
            "boolean" => Ok(ArcisType::Boolean),
            "string" => Ok(ArcisType::String),
            "void" => Ok(ArcisType::Void),
            other => Err(format!("unsupported type `{}` for the Cranelift backend", other)),
        },
        AstType::Literal(arcis_ast::LiteralValue::String(_)) => Ok(ArcisType::String),
        AstType::Literal(arcis_ast::LiteralValue::Number(_)) => Ok(ArcisType::Number),
        AstType::Literal(arcis_ast::LiteralValue::Bool(_)) => Ok(ArcisType::Boolean),
        // Unions/intersections erase to the first member's representation,
        // matching the Rust backend's erasure model.
        AstType::Union(members) | AstType::Intersection(members) => members
            .first()
            .map(|m| from_ast(m, enum_names))
            .unwrap_or(Ok(ArcisType::Void)),
        // Function values and named (type-alias / interface) types are not
        // yet lowered by the Cranelift backend; both need a runtime
        // representation beyond a plain i64 handle.
        AstType::Function { .. } => Err("function types are not yet supported by the Cranelift backend".to_string()),
        // Generics are Rust-backend-only (see `arcis-driver::build::check_no_generics`,
        // which rejects any generic construct before this backend ever
        // runs). This arm is defense-in-depth only, not the primary
        // guarantee — `from_ast`'s `Err`s get silently swallowed by some
        // callers (`.unwrap_or(ArcisType::Number)`), so it must never be
        // relied on alone.
        AstType::Generic { .. } => Err("generic types are not yet supported by the Cranelift backend — use --backend rust".to_string()),
        AstType::Named(other) => {
            // Could be an object type with a __Obj hash name, or a
            // user-defined interface/type-alias name. For now we accept
            // any named type as an opaque Object handle.
            if !other.is_empty() {
                Ok(ArcisType::Object)
            } else {
                Err(format!("unsupported type `{}` for the Cranelift backend", other))
            }
        }
    }
}
