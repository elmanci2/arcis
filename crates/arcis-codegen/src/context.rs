//! Shared context passed to every emitter.

use std::collections::{HashMap, HashSet};

use arcis_ast::Type;

/// Context passed to the emit functions: reassigned variables, the map of
/// declared types, the declared type of the let/const whose `ObjectLiteral`
/// we are currently emitting, and a flag indicating whether this is the root
/// `main` module (where object-type structs are defined) or a non-root
/// module (where they are referenced as `crate::__ObjNAME`).
pub(crate) struct Ctx<'a> {
    pub reassigned: &'a HashSet<String>,
    pub types: &'a HashMap<String, String>,
    /// Declared type of the let/const that contains the `ObjectLiteral`
    /// currently being emitted. `None` for free-floating expressions; in
    /// that case the codegen emits a `todo!()` (rustc will then report).
    pub current_let_type: Option<&'a Type>,
    /// `true` if we are generating the root (`main`) module.
    pub is_root: bool,
}