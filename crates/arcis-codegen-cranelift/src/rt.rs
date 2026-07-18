//! Cranelift declarations for every external function the lowered Arcis
//! program calls into. All of these live in [`crate::runtime::RUNTIME_C_SOURCE`]
//! (a `.c` file the driver compiles separately).
//!
//! Each field is a Cranelift `FuncId` — when we want to *call* the runtime
//! from inside a Cranelift function body, we look it up with
//! `module.declare_func_in_func(func_id, builder.func)` which gives us the
//! `FuncRef` to pass to `builder.ins().call(...)`.

use cranelift_codegen::ir::types::{F64, I32, I64};
use cranelift_codegen::ir::{AbiParam, Signature, Type};
use cranelift_module::{FuncId, Linkage, Module as CraneliftModule};
use cranelift_object::ObjectModule;

/// One-`FuncId`-per-runtime-function struct, declared once per module.
pub(crate) struct Runtime {
    // Phase 1: strings, printing, numbers.
    pub string_from_cstr: FuncId,
    pub string_concat: FuncId,
    pub string_eq: FuncId,
    pub string_drop: FuncId,
    pub print: FuncId,
    pub println: FuncId,
    pub num_to_string: FuncId,
    pub bool_to_string: FuncId,

    // Phase 2: string methods.
    pub string_to_uppercase: FuncId,
    pub string_to_lowercase: FuncId,
    pub string_trim: FuncId,
    pub string_substring: FuncId,
    pub string_index_of: FuncId,
    pub string_includes: FuncId,
    pub string_char_at: FuncId,

    // Phase 2: I/O.
    pub read_line: FuncId,

    // Phase 2: arrays.
    pub vec_new: FuncId,
    pub vec_push: FuncId,
    pub vec_pop: FuncId,
    pub vec_unshift: FuncId,
    pub vec_len: FuncId,
    pub vec_get: FuncId,
    pub vec_set: FuncId,
    #[allow(dead_code)]
    pub vec_drop: FuncId,

    // Phase 2: objects.
    pub object_new: FuncId,
    pub object_set: FuncId,
    pub object_get: FuncId,
    #[allow(dead_code)]
    pub object_drop: FuncId,
}

impl Runtime {
    pub(crate) fn declare(module: &mut ObjectModule) -> Result<Self, String> {
        let conv = module.target_config().default_call_conv;
        let build_sig = |params: &[Type], rets: &[Type]| {
            let mut s = Signature::new(conv);
            for p in params {
                s.params.push(AbiParam::new(*p));
            }
            for r in rets {
                s.returns.push(AbiParam::new(*r));
            }
            s
        };
        let mut decl = |name: &str, sig: Signature| {
            module
                .declare_function(name, Linkage::Import, &sig)
                .map_err(|e| format!("declare `{}`: {}", name, e))
        };

        // Phase 1
        let string_from_cstr = decl("arcis_string_from_cstr", build_sig(&[I64], &[I64]))?;
        let string_concat = decl("arcis_string_concat", build_sig(&[I64, I64], &[I64]))?;
        let string_eq = decl("arcis_string_eq", build_sig(&[I64, I64], &[I32]))?;
        let string_drop = decl("arcis_string_drop", build_sig(&[I64], &[]))?;
        let print = decl("arcis_print", build_sig(&[I64], &[]))?;
        let println = decl("arcis_println", build_sig(&[I64], &[]))?;
        let num_to_string = decl("arcis_num_to_string", build_sig(&[F64], &[I64]))?;
        let bool_to_string = decl("arcis_bool_to_string", build_sig(&[I32], &[I64]))?;

        // Phase 2: string methods  —  (ArcisString*) -> ArcisString*
        let string_to_uppercase = decl("arcis_string_to_uppercase", build_sig(&[I64], &[I64]))?;
        let string_to_lowercase = decl("arcis_string_to_lowercase", build_sig(&[I64], &[I64]))?;
        let string_trim = decl("arcis_string_trim", build_sig(&[I64], &[I64]))?;
        // (ArcisString*, int64_t, int64_t) -> ArcisString*
        let string_substring = decl("arcis_string_substring", build_sig(&[I64, I64, I64], &[I64]))?;
        // (ArcisString*, ArcisString*) -> f64
        let string_index_of = decl("arcis_string_index_of", build_sig(&[I64, I64], &[F64]))?;
        // (ArcisString*, ArcisString*) -> int32_t
        let string_includes = decl("arcis_string_includes", build_sig(&[I64, I64], &[I32]))?;
        // (ArcisString*, int64_t) -> ArcisString*
        let string_char_at = decl("arcis_string_char_at", build_sig(&[I64, I64], &[I64]))?;

        // Phase 2: I/O  —  () -> ArcisString*
        let read_line = decl("arcis_read_line", build_sig(&[], &[I64]))?;

        // Phase 2: arrays
        let vec_new = decl("arcis_vec_new", build_sig(&[], &[I64]))?;
        let vec_push = decl("arcis_vec_push", build_sig(&[I64, I64], &[]))?;
        let vec_pop = decl("arcis_vec_pop", build_sig(&[I64], &[I64]))?;
        let vec_unshift = decl("arcis_vec_unshift", build_sig(&[I64, I64], &[]))?;
        let vec_len = decl("arcis_vec_len", build_sig(&[I64], &[I32]))?;
        let vec_get = decl("arcis_vec_get", build_sig(&[I64, I32], &[I64]))?;
        let vec_set = decl("arcis_vec_set", build_sig(&[I64, I32, I64], &[]))?;
        let vec_drop = decl("arcis_vec_drop", build_sig(&[I64], &[]))?;

        // Phase 2: objects
        let object_new = decl("arcis_object_new", build_sig(&[], &[I64]))?;
        // (ArcisObject*, const char* key, int64_t value) -> void
        let object_set = decl("arcis_object_set", build_sig(&[I64, I64, I64], &[]))?;
        // (ArcisObject*, const char* key) -> int64_t value
        let object_get = decl("arcis_object_get", build_sig(&[I64, I64], &[I64]))?;
        let object_drop = decl("arcis_object_drop", build_sig(&[I64], &[]))?;

        Ok(Runtime {
            string_from_cstr,
            string_concat,
            string_eq,
            string_drop,
            print,
            println,
            num_to_string,
            bool_to_string,
            string_to_uppercase,
            string_to_lowercase,
            string_trim,
            string_substring,
            string_index_of,
            string_includes,
            string_char_at,
            read_line,
            vec_new,
            vec_push,
            vec_pop,
            vec_unshift,
            vec_len,
            vec_get,
            vec_set,
            vec_drop,
            object_new,
            object_set,
            object_get,
            object_drop,
        })
    }
}
