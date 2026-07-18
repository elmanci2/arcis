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
    pub string_from_cstr: FuncId,
    pub string_concat: FuncId,
    pub string_eq: FuncId,
    pub string_drop: FuncId,
    pub print: FuncId,
    pub println: FuncId,
    pub num_to_string: FuncId,
    pub bool_to_string: FuncId,
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

        // (const char*) -> ArcisString*        (i64 handle)
        let string_from_cstr = module
            .declare_function(
                "arcis_string_from_cstr",
                Linkage::Import,
                &build_sig(&[I64], &[I64]),
            )
            .map_err(|e| format!("declare `arcis_string_from_cstr`: {}", e))?;
        // (ArcisString*, ArcisString*) -> ArcisString*
        let string_concat = module
            .declare_function(
                "arcis_string_concat",
                Linkage::Import,
                &build_sig(&[I64, I64], &[I64]),
            )
            .map_err(|e| format!("declare `arcis_string_concat`: {}", e))?;
        // (ArcisString*, ArcisString*) -> int32_t  (0/1)
        let string_eq = module
            .declare_function(
                "arcis_string_eq",
                Linkage::Import,
                &build_sig(&[I64, I64], &[I32]),
            )
            .map_err(|e| format!("declare `arcis_string_eq`: {}", e))?;
        // (ArcisString*) -> void
        let string_drop = module
            .declare_function(
                "arcis_string_drop",
                Linkage::Import,
                &build_sig(&[I64], &[]),
            )
            .map_err(|e| format!("declare `arcis_string_drop`: {}", e))?;
        // (ArcisString*) -> void
        let print = module
            .declare_function(
                "arcis_print",
                Linkage::Import,
                &build_sig(&[I64], &[]),
            )
            .map_err(|e| format!("declare `arcis_print`: {}", e))?;
        let println = module
            .declare_function(
                "arcis_println",
                Linkage::Import,
                &build_sig(&[I64], &[]),
            )
            .map_err(|e| format!("declare `arcis_println`: {}", e))?;
        // (double) -> ArcisString*
        let num_to_string = module
            .declare_function(
                "arcis_num_to_string",
                Linkage::Import,
                &build_sig(&[F64], &[I64]),
            )
            .map_err(|e| format!("declare `arcis_num_to_string`: {}", e))?;
        // (int32_t) -> ArcisString*
        let bool_to_string = module
            .declare_function(
                "arcis_bool_to_string",
                Linkage::Import,
                &build_sig(&[I32], &[I64]),
            )
            .map_err(|e| format!("declare `arcis_bool_to_string`: {}", e))?;

        Ok(Runtime {
            string_from_cstr,
            string_concat,
            string_eq,
            string_drop,
            print,
            println,
            num_to_string,
            bool_to_string,
        })
    }
}
