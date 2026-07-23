//! Cranelift declarations for every external function the lowered Arcis
//! program calls into. All of these live in [`crate::runtime::RUNTIME_C_SOURCE`].

use cranelift_codegen::ir::types::{F64, I32, I64};
use cranelift_codegen::ir::{AbiParam, Signature, Type};
use cranelift_module::{FuncId, Linkage, Module as CraneliftModule};
use cranelift_object::ObjectModule;

pub(crate) struct Runtime {
    // Phase 1-2: strings, printing, numbers, arrays, objects, methods.
    pub string_from_cstr: FuncId, pub string_concat: FuncId, pub string_eq: FuncId,
    pub string_drop: FuncId, pub print: FuncId, pub println: FuncId,
    pub num_to_string: FuncId, pub bool_to_string: FuncId,
    pub parse_float: FuncId, pub is_nan: FuncId,
    pub string_to_uppercase: FuncId, pub string_to_lowercase: FuncId,
    pub string_trim: FuncId, pub string_substring: FuncId,
    pub string_index_of: FuncId, pub string_includes: FuncId, pub string_char_at: FuncId,
    pub read_line: FuncId,
    pub vec_new: FuncId, pub vec_push: FuncId, pub vec_pop: FuncId, pub vec_unshift: FuncId,
    pub vec_len: FuncId, pub vec_get: FuncId, pub vec_set: FuncId,
    pub vec_drop: FuncId, pub vec_extend: FuncId,
    pub object_new: FuncId, pub object_set: FuncId, pub object_get: FuncId, pub object_drop: FuncId,
    pub object_merge: FuncId,

    // Phase 5: sys.* — filesystem.
    pub fs_read_file: FuncId, pub fs_write_file: FuncId,
    pub fs_read_bytes: FuncId, pub fs_write_bytes: FuncId,
    pub fs_append_file: FuncId, pub fs_create_file: FuncId,
    pub fs_delete_file: FuncId, pub fs_delete_dir: FuncId, pub fs_delete_dir_all: FuncId,
    pub fs_mkdir: FuncId, pub fs_list_dir: FuncId,
    pub fs_copy: FuncId, pub fs_move: FuncId,

    // Phase 5: sys.* — path.
    pub path_exists: FuncId, pub path_is_file: FuncId, pub path_is_dir: FuncId,
    pub path_file_size: FuncId, pub path_file_info: FuncId,
    pub path_absolute: FuncId,

    // Phase 5: sys.* — process env.
    pub proc_current_dir: FuncId, pub proc_change_dir: FuncId,
    pub proc_temp_dir: FuncId, pub proc_home_dir: FuncId, pub proc_executable_path: FuncId,

    // Phase 5: sys.* — process management.
    pub process_current_pid: FuncId, pub process_parent_pid: FuncId,
    pub process_exec: FuncId, pub process_run: FuncId,
    pub process_spawn: FuncId, pub process_kill: FuncId, pub process_list: FuncId,

    // Phase 5: sys.* — env vars.
    pub env_get: FuncId, pub env_set: FuncId, pub env_delete: FuncId, pub env_all: FuncId,

    // Phase 5: sys.* — OS info.
    pub os_name: FuncId, pub os_version: FuncId, pub os_arch: FuncId,
    pub os_hostname: FuncId, pub os_username: FuncId, pub os_uptime: FuncId,
    pub os_locale: FuncId, pub os_cpu_count: FuncId,

    // Phase 5: sys.* — memory, cpu, gpu, disk, net.
    pub memory_total: FuncId, pub memory_free: FuncId, pub memory_used: FuncId, pub memory_available: FuncId,
    pub cpu_model: FuncId, pub cpu_brand: FuncId, pub cpu_frequency: FuncId, pub cpu_usage: FuncId, pub cpu_cores: FuncId,
    pub gpu_name: FuncId, pub gpu_list: FuncId,
    pub disk_free: FuncId,
    pub net_online: FuncId, pub net_public_ip: FuncId, pub net_interfaces: FuncId,

    // Phase 5: sys.args.
    pub sys_args: FuncId,

    // try / catch / throw. `raw_setjmp` is libc's real `setjmp` symbol,
    // called *directly* by the Cranelift-generated `try` code (never
    // through a C wrapper that returns) — see `runtime.rs`'s module doc
    // comment on why that distinction is load-bearing, not stylistic.
    pub try_push: FuncId, pub try_end: FuncId,
    pub throw: FuncId, pub thrown_value: FuncId,
    pub raw_setjmp: FuncId,
}

impl Runtime {
    pub(crate) fn declare(module: &mut ObjectModule) -> Result<Self, String> {
        let conv = module.target_config().default_call_conv;
        let sig = |params: &[Type], rets: &[Type]| {
            let mut s = Signature::new(conv);
            for p in params { s.params.push(AbiParam::new(*p)); }
            for r in rets { s.returns.push(AbiParam::new(*r)); }
            s
        };
        let mut decl = |name: &str, s: Signature| {
            module.declare_function(name, Linkage::Import, &s)
                .map_err(|e| format!("declare `{}`: {}", name, e))
        };

        // Macro-like helper to reduce repetition.
        macro_rules! d {
            ($name:expr, $params:expr, $rets:expr) => {
                decl($name, sig($params, $rets))?
            };
        }

        Ok(Runtime {
            string_from_cstr: d!("arcis_string_from_cstr", &[I64], &[I64]),
            string_concat:    d!("arcis_string_concat", &[I64, I64], &[I64]),
            string_eq:        d!("arcis_string_eq", &[I64, I64], &[I32]),
            string_drop:      d!("arcis_string_drop", &[I64], &[]),
            print:            d!("arcis_print", &[I64], &[]),
            println:          d!("arcis_println", &[I64], &[]),
            num_to_string:    d!("arcis_num_to_string", &[F64], &[I64]),
            bool_to_string:   d!("arcis_bool_to_string", &[I32], &[I64]),
            parse_float:      d!("arcis_parse_float", &[I64], &[F64]),
            is_nan:           d!("arcis_is_nan", &[F64], &[I32]),

            string_to_uppercase: d!("arcis_string_to_uppercase", &[I64], &[I64]),
            string_to_lowercase: d!("arcis_string_to_lowercase", &[I64], &[I64]),
            string_trim:         d!("arcis_string_trim", &[I64], &[I64]),
            string_substring:    d!("arcis_string_substring", &[I64, I64, I64], &[I64]),
            string_index_of:     d!("arcis_string_index_of", &[I64, I64], &[F64]),
            string_includes:     d!("arcis_string_includes", &[I64, I64], &[I32]),
            string_char_at:      d!("arcis_string_char_at", &[I64, I64], &[I64]),
            read_line:           d!("arcis_read_line", &[], &[I64]),

            vec_new:     d!("arcis_vec_new", &[], &[I64]),
            vec_push:    d!("arcis_vec_push", &[I64, I64], &[]),
            vec_pop:     d!("arcis_vec_pop", &[I64], &[I64]),
            vec_unshift: d!("arcis_vec_unshift", &[I64, I64], &[]),
            vec_len:     d!("arcis_vec_len", &[I64], &[I32]),
            vec_get:     d!("arcis_vec_get", &[I64, I32], &[I64]),
            vec_set:     d!("arcis_vec_set", &[I64, I32, I64], &[]),
            vec_drop:    d!("arcis_vec_drop", &[I64], &[]),
            vec_extend:  d!("arcis_vec_extend", &[I64, I64], &[]),

            object_new:  d!("arcis_object_new", &[], &[I64]),
            object_set:  d!("arcis_object_set", &[I64, I64, I64], &[]),
            object_get:  d!("arcis_object_get", &[I64, I64], &[I64]),
            object_drop: d!("arcis_object_drop", &[I64], &[]),
            object_merge: d!("arcis_object_merge", &[I64, I64], &[]),

            // sys.* — filesystem.
            fs_read_file:       d!("arcis_fs_read_file", &[I64], &[I64]),
            fs_write_file:      d!("arcis_fs_write_file", &[I64, I64], &[]),
            fs_read_bytes:      d!("arcis_fs_read_bytes", &[I64], &[I64]),
            fs_write_bytes:     d!("arcis_fs_write_bytes", &[I64, I64], &[]),
            fs_append_file:     d!("arcis_fs_append_file", &[I64, I64], &[]),
            fs_create_file:     d!("arcis_fs_create_file", &[I64], &[]),
            fs_delete_file:     d!("arcis_fs_delete_file", &[I64], &[]),
            fs_delete_dir:      d!("arcis_fs_delete_dir", &[I64], &[]),
            fs_delete_dir_all:  d!("arcis_fs_delete_dir_all", &[I64], &[]),
            fs_mkdir:           d!("arcis_fs_mkdir", &[I64], &[]),
            fs_list_dir:        d!("arcis_fs_list_dir", &[I64], &[I64]),
            fs_copy:            d!("arcis_fs_copy", &[I64, I64], &[]),
            fs_move:            d!("arcis_fs_move", &[I64, I64], &[]),

            // sys.* — path.
            path_exists:     d!("arcis_path_exists", &[I64], &[F64]),
            path_is_file:    d!("arcis_path_is_file", &[I64], &[F64]),
            path_is_dir:     d!("arcis_path_is_dir", &[I64], &[F64]),
            path_file_size:  d!("arcis_path_file_size", &[I64], &[F64]),
            path_file_info:  d!("arcis_path_file_info", &[I64], &[I64]),
            path_absolute:   d!("arcis_path_absolute", &[I64], &[I64]),

            // sys.* — process env.
            proc_current_dir:     d!("arcis_proc_current_dir", &[], &[I64]),
            proc_change_dir:      d!("arcis_proc_change_dir", &[I64], &[]),
            proc_temp_dir:        d!("arcis_proc_temp_dir", &[], &[I64]),
            proc_home_dir:        d!("arcis_proc_home_dir", &[], &[I64]),
            proc_executable_path: d!("arcis_proc_executable_path", &[], &[I64]),

            // sys.* — process management.
            process_current_pid:  d!("arcis_process_current_pid", &[], &[F64]),
            process_parent_pid:   d!("arcis_process_parent_pid", &[], &[F64]),
            process_exec:         d!("arcis_process_exec", &[I64, I64], &[I64]),
            process_run:          d!("arcis_process_run", &[I64], &[I64]),
            process_spawn:        d!("arcis_process_spawn", &[I64], &[F64]),
            process_kill:         d!("arcis_process_kill", &[F64], &[]),
            process_list:         d!("arcis_process_list", &[], &[I64]),

            // sys.* — env.
            env_get:    d!("arcis_env_get", &[I64], &[I64]),
            env_set:    d!("arcis_env_set", &[I64, I64], &[]),
            env_delete: d!("arcis_env_delete", &[I64], &[]),
            env_all:    d!("arcis_env_all", &[], &[I64]),

            // sys.* — OS.
            os_name:     d!("arcis_os_name", &[], &[I64]),
            os_version:  d!("arcis_os_version", &[], &[I64]),
            os_arch:     d!("arcis_os_arch", &[], &[I64]),
            os_hostname: d!("arcis_os_hostname", &[], &[I64]),
            os_username: d!("arcis_os_username", &[], &[I64]),
            os_uptime:   d!("arcis_os_uptime", &[], &[F64]),
            os_locale:   d!("arcis_os_locale", &[], &[I64]),
            os_cpu_count: d!("arcis_os_cpu_count", &[], &[F64]),

            // sys.* — memory, cpu, gpu, disk, net.
            memory_total:     d!("arcis_memory_total", &[], &[F64]),
            memory_free:      d!("arcis_memory_free", &[], &[F64]),
            memory_used:      d!("arcis_memory_used", &[], &[F64]),
            memory_available: d!("arcis_memory_available", &[], &[F64]),
            cpu_model:     d!("arcis_cpu_model", &[], &[I64]),
            cpu_brand:     d!("arcis_cpu_brand", &[], &[I64]),
            cpu_frequency: d!("arcis_cpu_frequency", &[], &[F64]),
            cpu_usage:     d!("arcis_cpu_usage", &[], &[F64]),
            cpu_cores:     d!("arcis_cpu_cores", &[], &[F64]),
            gpu_name:      d!("arcis_gpu_name", &[], &[I64]),
            gpu_list:      d!("arcis_gpu_list", &[], &[I64]),
            disk_free:     d!("arcis_disk_free", &[I64], &[F64]),
            net_online:    d!("arcis_net_online", &[], &[F64]),
            net_public_ip: d!("arcis_net_public_ip", &[], &[I64]),
            net_interfaces: d!("arcis_net_interfaces", &[], &[I64]),

            sys_args: d!("arcis_sys_args", &[], &[I64]),

            try_push:     d!("arcis_try_push", &[], &[I64]),
            try_end:      d!("arcis_try_end", &[], &[]),
            throw:        d!("arcis_throw", &[I64], &[]),
            thrown_value: d!("arcis_thrown_value", &[], &[I64]),
            // libc's own `setjmp(jmp_buf)`, declared and called directly —
            // NOT a wrapper we define in `arcis_runtime.c`, resolved by the
            // final `cc` link step against the system libc.
            raw_setjmp:   d!("setjmp", &[I64], &[I32]),
        })
    }
}
