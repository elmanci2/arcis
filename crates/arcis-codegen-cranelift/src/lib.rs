//! Arcis code generator: AST → native object files via Cranelift.
//!
//! This crate is the second backend for the Arcis compiler. The original
//! [`arcis_codegen`] crate lowers `.tsr` source to **Rust source** and then
//! delegates to `rustc`; this crate lowers the same AST directly to
//! **Cranelift IR** and emits machine code (one `.o` per module) that can be
//! linked with the system `cc` against a small C runtime. End result: an
//! Arcis user who installs the compiler never needs Rust to build an Arcis
//! program.
//!
//! Public surface:
//! - [`compile_to_object`]: returns one `.o` per module, all sitting in the
//!   provided output directory.
//! - [`runtime::RUNTIME_C_SOURCE`]: the C source of the helper functions that
//!   the generated code calls into. Callers are expected to compile this
//!   with `cc -c arcis_runtime.c -o arcis_runtime.o` and link the resulting
//!   `.o` together with the per-module object files from `compile_to_object`.
//!
//! ## ABI / runtime contract
//!
//! `ArcisString` values cross the runtime boundary as opaque `int64_t`
//! handles (pointers to a heap-allocated record). See [`runtime`] for
//! details and the list of exported C functions.
//!
//! ## Scope
//!
//! Phase 1 (this iteration): numbers, booleans, strings, `let`/`const`,
//! reassignment, `if`/`else`/`while`/`for`, all 13 binary operators,
//! unary `!`/`-`, function declarations + calls, `print(…)`, single-file
//! programs. Phases 2–6 (arrays, methods, modules, `sys.*`, full parity)
//! come in subsequent sessions.

#![allow(clippy::all)]

pub mod runtime;

mod builtin;
mod collect;
mod context;
mod expr;
mod function;
mod method;
mod module;
mod rt;
mod stmt;
mod sys;
mod types;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arcis_linker::Module;
use cranelift_codegen::isa::TargetIsa;
use cranelift_codegen::settings::Configurable;
use cranelift_module::Module as CraneliftModule;
use cranelift_object::{ObjectBuilder, ObjectModule, ObjectProduct};
use target_lexicon::Triple;

/// Compile every Arcis module in `modules` to a native object file inside
/// `output_dir`. Returns the path of every generated `.o` in the same order
/// as the input modules (entry first).
///
/// The runtime object (`arcis_runtime.o`) is **not** produced here — it is
/// the caller's responsibility to compile [`runtime::RUNTIME_C_SOURCE`] and
/// link it together with the per-module object files.
///
/// For Phase 1, single-file projects, only the root module produces real
/// code; the remaining `.o` files are placeholders (still emitted so that
/// the linker has the correct layout to reject or ignore).
pub fn compile_to_object(
    modules: &[Module],
    output_dir: &Path,
    host_triple: &Triple,
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("could not create output dir `{}`: {}", output_dir.display(), e))?;
    let isa = build_isa(host_triple)?;

    let mut paths = Vec::with_capacity(modules.len());
    for (i, m) in modules.iter().enumerate() {
        let product = compile_module(m, isa.clone(), i == 0)?;
        let bytes = product
            .emit()
            .map_err(|e| format!("could not emit object for module `{}`: {}", m.id, e))?;
        let path = output_dir.join(format!("{}.o", m.id));
        std::fs::write(&path, bytes)
            .map_err(|e| format!("could not write `{}`: {}", path.display(), e))?;
        paths.push(path);
    }
    Ok(paths)
}

/// Build a Cranelift target-ISA from a host triple, configured for
/// non-position-independent flat binaries (matches the C runtime's
/// expectation of a `_start` symbol).
fn build_isa(host_triple: &Triple) -> Result<Arc<dyn TargetIsa>, String> {
    let mut cfg = cranelift_codegen::settings::builder();
    cfg.set("use_colocated_libcalls", "false")
        .map_err(|e| format!("settings: {}", e))?;
    cfg.set("is_pic", "false")
        .map_err(|e| format!("settings: {}", e))?;
    let flags = cranelift_codegen::settings::Flags::new(cfg);
    cranelift_codegen::isa::lookup(host_triple.clone())
        .map_err(|e| format!("Cranelift ISA lookup failed: {}", e))?
        .finish(flags)
        .map_err(|e| format!("Cranelift ISA setup failed: {}", e))
}

/// Compile a single Arcis module to one in-memory object product.
fn compile_module(
    m: &Module,
    isa: Arc<dyn TargetIsa>,
    is_root: bool,
) -> Result<ObjectProduct, String> {
    let id_bytes = m.id.as_bytes().to_vec();
    let builder = ObjectBuilder::new(isa, id_bytes, cranelift_module::default_libcall_names())
        .map_err(|e| format!("ObjectBuilder: {}", e))?;
    let mut module = ObjectModule::new(builder);
    module::emit(&mut module, m, is_root)?;
    Ok(module.finish())
}
