//! Dispatch for `sys.*` builtins (Phase 5).

use std::collections::HashMap;
use arcis_ast::Expr;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::InstBuilder;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{FuncId, Module as CraneliftModule};
use cranelift_object::ObjectModule;
use crate::context::{FnInfo, FunctionCtx};
use crate::expr;
use crate::rt::Runtime;
use crate::types::ArcisType;

type V = cranelift_codegen::ir::Value;

/// Dispatch `sys.X(args)`.
pub(crate) fn try_emit_call(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    property: &str,
    args: &[Expr],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<Option<(V, ArcisType)>, String> {
    // Helper macros to reduce boilerplate.
    macro_rules! c0s { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); (builder.inst_results(cl)[0], ArcisType::String) }}; }
    macro_rules! c0n { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); (builder.inst_results(cl)[0], ArcisType::Number) }}; }
    macro_rules! c0a { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); (builder.inst_results(cl)[0], ArcisType::Array) }}; }
    macro_rules! c0b { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); let raw=builder.inst_results(cl)[0]; let z=builder.ins().f64const(0.0); (builder.ins().fcmp(cranelift_codegen::ir::condcodes::FloatCC::NotEqual, raw, z), ArcisType::Boolean) }}; }
    macro_rules! c1b { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); let raw=builder.inst_results(cl)[0]; let z=builder.ins().f64const(0.0); (builder.ins().fcmp(cranelift_codegen::ir::condcodes::FloatCC::NotEqual, raw, z), ArcisType::Boolean) }}; }
    macro_rules! c1s { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); (builder.inst_results(cl)[0], ArcisType::String) }}; }
    macro_rules! c1n { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); (builder.inst_results(cl)[0], ArcisType::Number) }}; }
    macro_rules! c1a { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); (builder.inst_results(cl)[0], ArcisType::Array) }}; }
    macro_rules! c1o { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); (builder.inst_results(cl)[0], ArcisType::Object) }}; }
    // `cmd + optional args-array` call forms: `sys.exec("ls")` and
    // `sys.exec("ls", ["-la"])` both hit the same 2-arg runtime function;
    // a missing array is passed as a NULL (0) handle.
    macro_rules! c2opt { ($f:ident, $args:expr, $ret:expr) => {{
        let (v0,_)=expr::emit(builder,fctx,&$args[0],runtime,user_fns,module)?;
        let v1 = if $args.len() >= 2 {
            let (v,_)=expr::emit(builder,fctx,&$args[1],runtime,user_fns,module)?; v
        } else {
            builder.ins().iconst(I64, 0)
        };
        let c=module.declare_func_in_func(runtime.$f,builder.func);
        let cl=builder.ins().call(c,&[v0,v1]);
        (builder.inst_results(cl)[0], $ret)
    }}; }
    macro_rules! c1v { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); builder.ins().call(c,&[v]); (builder.ins().iconst(I64,0), ArcisType::Void) }}; }
    macro_rules! c2v { ($f:ident, $a:expr, $b:expr) => {{ let (v0,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let (v1,_)=expr::emit(builder,fctx,$b,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); builder.ins().call(c,&[v0,v1]); (builder.ins().iconst(I64,0), ArcisType::Void) }}; }

    let r: Option<(V, ArcisType)> = match property {
        "readFile"     if args.len()==1 => Some(c1s!(fs_read_file, &args[0])),
        "readBytes"    if args.len()==1 => Some(c1s!(fs_read_bytes, &args[0])),
        "writeFile"    if args.len()==2 => Some(c2v!(fs_write_file, &args[0], &args[1])),
        "writeBytes"   if args.len()==2 => Some(c2v!(fs_write_bytes, &args[0], &args[1])),
        "appendFile"   if args.len()==2 => Some(c2v!(fs_append_file, &args[0], &args[1])),
        "createFile"   if args.len()==1 => Some(c1v!(fs_create_file, &args[0])),
        "deleteFile"   if args.len()==1 => Some(c1v!(fs_delete_file, &args[0])),
        "deleteDir"    if args.len()==1 => Some(c1v!(fs_delete_dir, &args[0])),
        "deleteDirAll" if args.len()==1 => Some(c1v!(fs_delete_dir_all, &args[0])),
        "mkdir"        if args.len()==1 => Some(c1v!(fs_mkdir, &args[0])),
        "listDir"      if args.len()==1 => Some(c1a!(fs_list_dir, &args[0])),
        "copy"         if args.len()==2 => Some(c2v!(fs_copy, &args[0], &args[1])),
        "move" | "rename" if args.len()==2 => Some(c2v!(fs_move, &args[0], &args[1])),

        "exists"    if args.len()==1 => Some(c1b!(path_exists, &args[0])),
        "isFile"    if args.len()==1 => Some(c1b!(path_is_file, &args[0])),
        "isDir"     if args.len()==1 => Some(c1b!(path_is_dir, &args[0])),
        "fileSize"  if args.len()==1 => Some(c1n!(path_file_size, &args[0])),
        "fileInfo"  if args.len()==1 => Some(c1s!(path_file_info, &args[0])),
        "absolute"  if args.len()==1 => Some(c1s!(path_absolute, &args[0])),
        "relative"  if args.len()==1 => Some(c1s!(path_absolute, &args[0])), // stub: return absolute
        "createSymlink" if args.len()==2 => Some(c2v!(fs_copy, &args[0], &args[1])), // stub
        "readLink" if args.len()==1 => Some(c1s!(path_absolute, &args[0])), // stub: return absolute

        "currentDir"     if args.is_empty() => Some(c0s!(proc_current_dir)),
        "tempDir"        if args.is_empty() => Some(c0s!(proc_temp_dir)),
        "homeDir"        if args.is_empty() => Some(c0s!(proc_home_dir)),
        "executablePath" if args.is_empty() => Some(c0s!(proc_executable_path)),
        "changeDir"      if args.len()==1 => Some(c1v!(proc_change_dir, &args[0])),

        "currentPid" if args.is_empty() => Some(c0n!(process_current_pid)),
        "parentPid"  if args.is_empty() => Some(c0n!(process_parent_pid)),
        "exec"       if args.len()>=1   => Some(c2opt!(process_exec, args, ArcisType::String)),
        "process"    if args.len()>=1   => Some(c2opt!(process_run_obj, args, ArcisType::Object)),
        "spawn"      if args.len()>=1   => Some(c2opt!(process_spawn, args, ArcisType::Number)),
        "kill"       if args.len()==1   => Some(c1v!(process_kill, &args[0])),
        "processes"  if args.is_empty() => Some(c0a!(process_list)),

        _ => None,
    };
    Ok(r)
}

/// Dispatch `sys.ns.method(args)`.
pub(crate) fn try_emit_subns(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    ns: &str,
    method: &str,
    args: &[Expr],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<Option<(V, ArcisType)>, String> {
    macro_rules! c0s { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); (builder.inst_results(cl)[0], ArcisType::String) }}; }
    macro_rules! c0n { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); (builder.inst_results(cl)[0], ArcisType::Number) }}; }
    macro_rules! c0a { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); (builder.inst_results(cl)[0], ArcisType::Array) }}; }
    macro_rules! c0b { ($f:ident) => {{ let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[]); let raw=builder.inst_results(cl)[0]; let z=builder.ins().f64const(0.0); (builder.ins().fcmp(cranelift_codegen::ir::condcodes::FloatCC::NotEqual, raw, z), ArcisType::Boolean) }}; }
    macro_rules! c1b { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); let raw=builder.inst_results(cl)[0]; let z=builder.ins().f64const(0.0); (builder.ins().fcmp(cranelift_codegen::ir::condcodes::FloatCC::NotEqual, raw, z), ArcisType::Boolean) }}; }
    macro_rules! c1s { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); (builder.inst_results(cl)[0], ArcisType::String) }}; }
    macro_rules! c1n { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); let cl=builder.ins().call(c,&[v]); (builder.inst_results(cl)[0], ArcisType::Number) }}; }
    macro_rules! c2v { ($f:ident, $a:expr, $b:expr) => {{ let (v0,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let (v1,_)=expr::emit(builder,fctx,$b,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); builder.ins().call(c,&[v0,v1]); (builder.ins().iconst(I64,0), ArcisType::Void) }}; }
    macro_rules! c1v { ($f:ident, $a:expr) => {{ let (v,_)=expr::emit(builder,fctx,$a,runtime,user_fns,module)?; let c=module.declare_func_in_func(runtime.$f,builder.func); builder.ins().call(c,&[v]); (builder.ins().iconst(I64,0), ArcisType::Void) }}; }

    let r: Option<(V, ArcisType)> = match (ns, method) {
        ("env", "get")    if args.len()==1 => Some(c1s!(env_get, &args[0])),
        ("env", "set")    if args.len()==2 => Some(c2v!(env_set, &args[0], &args[1])),
        ("env", "delete") if args.len()==1 => Some(c1v!(env_delete, &args[0])),
        ("env", "all")    if args.is_empty() => Some(c0a!(env_all)),

        ("os", "name")     if args.is_empty() => Some(c0s!(os_name)),
        ("os", "version")  if args.is_empty() => Some(c0s!(os_version)),
        ("os", "arch")     if args.is_empty() => Some(c0s!(os_arch)),
        ("os", "hostname") if args.is_empty() => Some(c0s!(os_hostname)),
        ("os", "username") if args.is_empty() => Some(c0s!(os_username)),
        ("os", "uptime")   if args.is_empty() => Some(c0n!(os_uptime)),
        ("os", "locale")   if args.is_empty() => Some(c0s!(os_locale)),
        ("os", "cpuCount") if args.is_empty() => Some(c0n!(os_cpu_count)),

        ("memory", "total")     if args.is_empty() => Some(c0n!(memory_total)),
        ("memory", "free")      if args.is_empty() => Some(c0n!(memory_free)),
        ("memory", "used")      if args.is_empty() => Some(c0n!(memory_used)),
        ("memory", "available") if args.is_empty() => Some(c0n!(memory_available)),

        ("cpu", "model")     if args.is_empty() => Some(c0s!(cpu_model)),
        ("cpu", "brand")     if args.is_empty() => Some(c0s!(cpu_brand)),
        ("cpu", "frequency") if args.is_empty() => Some(c0n!(cpu_frequency)),
        ("cpu", "usage")     if args.is_empty() => Some(c0n!(cpu_usage)),
        ("cpu", "cores")     if args.is_empty() => Some(c0n!(cpu_cores)),

        ("gpu", "list")   if args.is_empty() => Some(c0a!(gpu_list)),
        ("gpu", "name")   if args.is_empty() => Some(c0s!(cpu_model)), // reuse cpu model as gpu name stub
        ("gpu", "vendor") if args.is_empty() => Some(c0s!(cpu_brand)), // reuse cpu brand as gpu vendor stub
        ("gpu", "memory") if args.is_empty() => Some(c0n!(memory_total)), // stub

        ("disk", "free")  if args.len()==1 => Some(c1n!(disk_free, &args[0])),
        ("disk", "list")  if args.is_empty() => Some(c0a!(gpu_list)), // stub for now
        ("disk", "used")  if args.len()==1 => Some(c1n!(disk_free, &args[0])), // stub
        ("disk", "total") if args.len()==1 => Some(c1n!(disk_free, &args[0])), // stub

        ("net", "online")     if args.is_empty() => Some(c0b!(net_online)),
        ("net", "publicIp")   if args.is_empty() => Some(c0s!(net_public_ip)),
        ("net", "interfaces") if args.is_empty() => Some(c0a!(net_interfaces)),
        ("net", "hostname")   if args.is_empty() => Some(c0s!(os_hostname)),
        ("net", "ip")         if args.is_empty() => Some(c0s!(net_public_ip)), // stub

        _ => None,
    };
    Ok(r)
}

/// Dispatch `sys.property` member access.
pub(crate) fn try_emit_member(
    builder: &mut FunctionBuilder,
    _fctx: &mut FunctionCtx,
    property: &str,
    runtime: &Runtime,
    _user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<Option<(V, ArcisType)>, String> {
    match property {
        "args" => {
            let c = module.declare_func_in_func(runtime.sys_args, builder.func);
            let call = builder.ins().call(c, &[]);
            let h = builder.inst_results(call)[0];
            Ok(Some((h, ArcisType::Array)))
        }
        "gpu" => {
            let c = module.declare_func_in_func(runtime.gpu_name, builder.func);
            let call = builder.ins().call(c, &[]);
            let h = builder.inst_results(call)[0];
            Ok(Some((h, ArcisType::String)))
        }
        _ => Ok(None),
    }
}
