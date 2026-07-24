//! Shared context passed to every emitter.

use std::collections::{HashMap, HashSet};

use arcis_ast::Type;

/// Context passed to the emit functions: reassigned variables, the map of
/// declared types, the declared type of the let/const whose `ObjectLiteral`
/// we are currently emitting, the function's return type (when we are
/// inside a function body), and a flag indicating whether this is the root
/// `main` module (where object-type structs are defined) or a non-root
/// module (where they are referenced as `crate::__ObjNAME`).
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub reassigned: &'a HashSet<String>,
    pub types: &'a HashMap<String, String>,
    /// Declared type of the let/const that contains the `ObjectLiteral`
    /// currently being emitted. `None` for free-floating expressions; in
    /// that case the codegen emits a `todo!()` (rustc will then report).
    pub current_let_type: Option<&'a Type>,
    /// Return type of the function we are currently inside. Only set when
    /// emitting a function body; `None` elsewhere. When `Stmt::Return`
    /// emits its value, this type is consulted so that
    /// `return { ... };` can resolve inline object literals against the
    /// function's declared return shape.
    pub current_return_type: Option<&'a Type>,
    /// `true` if we are generating the root (`main`) module.
    pub is_root: bool,
    /// Names of every `enum` declared anywhere in the program. Used by
    /// `emit_member` to tell `Color.Red` (an enum variant, -> `Color::Red`)
    /// apart from `person.name` (a field access).
    pub enum_names: &'a HashSet<String>,
    /// Local names bound by namespace imports (`import utils;` /
    /// `import utils as u;` / `import * as u from "utils";`). Used by
    /// `emit_member` to emit `u::item` instead of `u.item`.
    pub namespace_names: &'a HashSet<String>,
    /// Cross-module function/const type facts (same [`arcis_validation::TypeEnv`]
    /// the driver used for inference and the null-safety checker).
    pub env: &'a arcis_validation::TypeEnv,
    /// Flat name → `Type` scope for this module's own locals/params (see
    /// `collect::collect_type_scope`). Paired with `env`, lets codegen ask
    /// `arcis_validation::expr_type` whether an arbitrary expression
    /// (not just a bare identifier) is optional — used to auto-wrap a
    /// `return`ed definite value in `Some(...)` when the function's return
    /// type is `T?`, and to `.unwrap()` a call result under `!`.
    pub type_scope: &'a HashMap<String, Type>,
    /// struct name → field shape, for every `Type::Object` collected across
    /// the program (interfaces, object-shaped type aliases, inline object
    /// types). A generic interface/alias usage site is `Type::Generic {
    /// name, args }` (never inlined back to `Type::Object` — see
    /// `arcis-codegen::collect::substitute_named`'s doc comment), so object-
    /// literal emission needs this side lookup to find the field TEMPLATE
    /// (bare type-param types like `T`) by name; the concrete `args` are
    /// left for `rustc`'s own inference to fill in from the `let`'s
    /// declared type, same as a non-generic struct literal already relies
    /// on context to fill in its concrete field types.
    pub struct_fields: &'a HashMap<String, Vec<(String, Box<Type>, bool)>>,
}