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
    /// Names that the pre-pass flagged as reassigned. Reserved for future
    /// use (e.g. marking runtime allocations as mutable).
    #[allow(dead_code)]
    reassigned: HashSet<String>,
    /// Stack of nested loops.
    loops: Vec<LoopFrame>,
    /// Map from literal bytes → Cranelift `DataId`. Populated lazily on
    /// first sight by [`crate::expr::emit`] for `Expr::String`.
    pub string_literals: HashMap<Vec<u8>, DataId>,
    /// Counter used to mint unique data-object names.
    pub string_literal_counter: u32,
    /// Counter used to mint unique `Variable` indices.
    var_counter: u32,
}

impl FunctionCtx {
    pub fn new(reassigned: &HashSet<String>) -> Self {
        Self {
            vars: HashMap::new(),
            currents: HashMap::new(),
            types: HashMap::new(),
            reassigned: reassigned.clone(),
            loops: Vec::new(),
            string_literals: HashMap::new(),
            string_literal_counter: 0,
            var_counter: 0,
        }
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
