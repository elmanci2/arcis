//! Per-function state shared between `stmt::emit` and `expr::emit`.
//!
//! Tracks:
//! - the Cranelift `Variable` for each Arcis binding (one per `let`/`const`),
//! - the current SSA value for each binding (so reads after assignments
//!   return the most-recent edge),
//! - the declared Arcis type of each binding (drives both IR typing and
//!   plus-disambiguation: `+` is numeric on F64 and string concatenation
//!   on I64 handles),
//! - the loop stack so `break` / `continue` inside a nested loop target the
//!   right exit block,
//! - a map from string-literal bytes → Cranelift `DataId`, so each program
//!   only emits one data object per unique literal.

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::{Block, Value};
use cranelift_codegen::entity::EntityRef;
use cranelift_frontend::Variable;
use cranelift_module::DataId;

use crate::types::ArcisType;

/// Info about a user-defined function available for calling.
#[derive(Debug, Clone)]
pub(crate) struct FnInfo {
    pub id: cranelift_module::FuncId,
    pub params: Vec<ArcisType>,
    /// Declared Arcis return type — the machine signature alone can't
    /// distinguish `string` / `array` / `object` (all `I64` handles).
    pub ret: ArcisType,
}

/// Stack frame for one enclosing loop. Used by `stmt::emit` to dispatch
/// `break` and `continue` to the right Cranelift block.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LoopFrame {
    /// Block where `continue` lands.
    pub continue_target: Block,
    /// Block where `break` lands.
    pub break_target: Block,
    /// Block that comes after the loop (the natural fall-through). May be
    /// the same as `break_target` for the outermost loop.
    pub after: Block,
    /// `FunctionCtx::open_try_count` at the moment this loop was entered —
    /// `break`/`continue` need to call `arcis_try_end()` for every `try`
    /// opened *since* (the delta), not every `try` open at all, so a `try`
    /// wrapping the whole loop stays open after a `break`.
    pub try_depth_at_entry: u32,
}

/// State carried inside a single Cranelift function body while
/// `stmt::emit` / `expr::emit` walk the AST.
pub(crate) struct FunctionCtx {
    /// Variable declarations (one Cranelift `Variable` per Arcis binding).
    vars: HashMap<String, Variable>,
    /// Current SSA value for each binding (updated on assignment).
    currents: HashMap<String, Value>,
    /// The declared Arcis type of each binding.
    types: HashMap<String, ArcisType>,
    /// For array bindings: the ArcisType of each element.
    element_types: HashMap<String, ArcisType>,
    /// For object bindings: field-name → ArcisType maps, keyed by variable name.
    object_field_types: HashMap<String, HashMap<String, ArcisType>>,
    /// Names that the pre-pass flagged as reassigned. Reserved for future
    /// use (e.g. marking runtime allocations as mutable).
    #[allow(dead_code)]
    reassigned: HashSet<String>,
    /// enum name -> (variant name -> numeric value), from `collect::collect_enums`.
    enums: HashMap<String, HashMap<String, f64>>,
    /// `enums.keys()`, cached so `types::from_ast` callers that only have
    /// `fctx` (not the raw `enums` map) can borrow a `&HashSet<String>`.
    enum_names: HashSet<String>,
    /// Stack of nested loops.
    loops: Vec<LoopFrame>,
    /// Map from literal bytes → Cranelift `DataId`. Populated lazily on
    /// first sight by [`crate::expr::emit`] for `Expr::String`.
    pub string_literals: HashMap<Vec<u8>, DataId>,
    /// Counter used to mint unique data-object names.
    pub string_literal_counter: u32,
    /// Counter used to mint unique `Variable` indices.
    var_counter: u32,
    /// Counter used to mint unique names for inline arrow functions
    /// lambda-lifted on the spot (`arr.map(x => x*2)`) — see `method.rs`.
    pub(crate) arrow_counter: u32,
    /// How many `try` blocks are currently open (lexically) at the current
    /// codegen position. `Stmt::Return`/`Break`/`Continue` need this to
    /// call `arcis_try_end()` the right number of times before an early
    /// exit, keeping the C runtime's `arcis_try_depth` balanced.
    pub(crate) open_try_count: u32,
    /// Module-level constants visible to this function: the module's own
    /// top-level `let`/`const` initializers (non-root modules) plus every
    /// imported constant (`from mate import PI`). An identifier that isn't
    /// a local binding re-emits the initializer expression in place —
    /// Cranelift modules have no linkable data symbols for consts yet.
    pub(crate) global_consts: HashMap<String, arcis_ast::Expr>,
    /// Program-wide fallback: field name → ArcisType, collected from every
    /// object type / interface in every module. Used to type a field access
    /// whose receiver has no per-variable tracked shape (nested members,
    /// call results). Last-write-wins on cross-interface collisions — only
    /// consulted when the alternative is a silent f64 reinterpretation.
    pub(crate) global_field_types: HashMap<String, ArcisType>,
    /// The enclosing function's declared return representation. `Stmt::Return`
    /// needs this to coerce a literal `null`/`undefined` (which emits as a
    /// dummy `ArcisType::Void` value with no meaningful bits) into the real
    /// "missing" sentinel of the function's ACTUAL Cranelift return type —
    /// returning the raw dummy value would otherwise produce a return value
    /// of the wrong Cranelift IR type (verifier error) whenever the return
    /// type isn't itself `Void`.
    pub(crate) return_ty: ArcisType,
}

impl FunctionCtx {
    pub fn new(reassigned: &HashSet<String>, enums: &HashMap<String, HashMap<String, f64>>) -> Self {
        Self {
            vars: HashMap::new(),
            currents: HashMap::new(),
            types: HashMap::new(),
            element_types: HashMap::new(),
            object_field_types: HashMap::new(),
            reassigned: reassigned.clone(),
            enums: enums.clone(),
            enum_names: enums.keys().cloned().collect(),
            loops: Vec::new(),
            string_literals: HashMap::new(),
            string_literal_counter: 0,
            var_counter: 0,
            arrow_counter: 0,
            open_try_count: 0,
            global_consts: HashMap::new(),
            global_field_types: HashMap::new(),
            return_ty: ArcisType::Void,
        }
    }

    /// The full enum table (name -> variant -> value), for lambda-lifting
    /// an inline arrow on the spot (`method.rs`) — its body may itself
    /// reference an enum, so the synthetic function needs the same table.
    pub(crate) fn enums(&self) -> &HashMap<String, HashMap<String, f64>> {
        &self.enums
    }

    /// `true` if `name` is a declared `enum`.
    pub(crate) fn is_enum_name(&self, name: &str) -> bool {
        self.enums.contains_key(name)
    }

    /// The numeric value of `enum_name.variant_name`, if both exist.
    pub(crate) fn enum_variant_value(&self, enum_name: &str, variant_name: &str) -> Option<f64> {
        self.enums.get(enum_name)?.get(variant_name).copied()
    }

    /// Every declared enum's name, for `types::from_ast` callers that only
    /// have `fctx` in scope.
    pub(crate) fn enum_names(&self) -> &HashSet<String> {
        &self.enum_names
    }

    pub fn define(&mut self, name: &str, ty: ArcisType, init: Value, builder: &mut cranelift_frontend::FunctionBuilder<'_>) -> Variable {
        let var = Variable::new(self.var_counter as usize);
        self.var_counter += 1;
        builder.declare_var(var, ty.to_cl());
        builder.def_var(var, init);
        self.vars.insert(name.to_string(), var);
        self.currents.insert(name.to_string(), init);
        self.types.insert(name.to_string(), ty);
        var
    }

    pub fn read(&self, name: &str) -> Option<Value> {
        self.currents.get(name).copied()
    }

    /// Look up the Cranelift `Variable` for an Arcis binding. The frontend
    /// needs this to call `use_var` (which inserts phi nodes at merge
    /// points); [`read`] only returns the SSA value we last saw, which
    /// is wrong across block merges.
    pub fn var(&self, name: &str) -> Option<&cranelift_frontend::Variable> {
        self.vars.get(name)
    }

    pub fn ty(&self, name: &str) -> Option<ArcisType> {
        self.types.get(name).copied()
    }

    /// The element type of an array binding (or None if not an array).
    pub(crate) fn element_ty(&self, name: &str) -> Option<ArcisType> {
        self.element_types.get(name).copied()
    }

    /// Set the element type for an array binding.
    pub(crate) fn set_element_ty(&mut self, name: &str, elem_ty: ArcisType) {
        self.element_types.insert(name.to_string(), elem_ty);
    }

    /// Register an object's field types from its Type annotation.
    pub(crate) fn set_object_fields(&mut self, name: &str, fields: &[(String, Box<arcis_ast::Type>, bool)]) {
        let mut map = HashMap::new();
        for (fname, fty, _optional) in fields {
            if let Ok(at) = crate::types::from_ast(fty, &self.enum_names) {
                map.insert(fname.clone(), at);
            }
        }
        self.object_field_types.insert(name.to_string(), map);
    }

    /// Look up the ArcisType of an object's field.
    pub(crate) fn object_field_ty(&self, obj_name: &str, field_name: &str) -> Option<ArcisType> {
        let r = self.object_field_types.get(obj_name).and_then(|m| m.get(field_name).copied());
        if std::env::var("ARCIS_DEBUG_SHAPES").is_ok() {
            eprintln!("[shape] lookup {}.{} -> {:?} (known: {:?})", obj_name, field_name, r, self.object_field_types.keys().collect::<Vec<_>>());
        }
        r
    }

    /// The initializer expression of a module-level or imported constant.
    pub(crate) fn global_const(&self, name: &str) -> Option<&arcis_ast::Expr> {
        self.global_consts.get(name)
    }

    /// Copy the tracked field-type map of `src` onto `dst`. Used to give a
    /// `for (x of arr)` loop variable (or any derived binding) the same
    /// object shape as its source array binding.
    pub(crate) fn copy_object_fields(&mut self, src: &str, dst: &str) {
        if let Some(map) = self.object_field_types.get(src).cloned() {
            self.object_field_types.insert(dst.to_string(), map);
        }
    }

    pub fn rebind(&mut self, name: &str, value: Value, builder: &mut cranelift_frontend::FunctionBuilder<'_>) {
        if let Some(&var) = self.vars.get(name) {
            builder.def_var(var, value);
            self.currents.insert(name.to_string(), value);
        }
    }

    pub fn push_loop(&mut self, frame: LoopFrame) {
        self.loops.push(frame);
    }

    pub fn pop_loop(&mut self) -> Option<LoopFrame> {
        self.loops.pop()
    }

    pub fn current_loop(&self) -> Option<&LoopFrame> {
        self.loops.last()
    }
}
