//! Shared context passed to every emitter.

use std::collections::{HashMap, HashSet};

use arcis_ast::Type;

/// Context passed to the emit functions: reassigned variables, the map of
/// declared types, the declared type of the let/const whose `ObjectLiteral`
/// we are currently emitting, the function's return type (when we are
/// inside a function body), and a flag indicating whether this is the root
/// `main` module (where object-type structs are defined) or a non-root
/// module (where they are referenced as `crate::__ObjNAME`).
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
}