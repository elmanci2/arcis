//! Static table of every completion / hover candidate.
//!
//! Built once at startup, used by [`crate::completion`] and
//! [`crate::hover`]. The table is hand-maintained — keeping it
//! in one place (instead of generating from `arcis-codegen`) makes it
//! trivial to tweak documentation strings without rebuilding the
//! codegen crate.
//!
//! ## Categories
//!
//! - [`KEYWORDS`] — reserved Arcis keywords + the four primitive type
//!   names (`string`/`number`/`boolean`/`void`). Always offered.
//! - [`TOP_LEVEL_BUILTINS`] — the global functions (`print`, `input`)
//!   and every top-level `sys.X` builtin. Offered at statement start
//!   and when typing `sys.`.

use crate::lsp::CompletionItemKind;

/// One entry in the static builtin table.
#[derive(Debug, Clone, Copy)]
pub struct Builtin {
    /// What the editor inserts (e.g. `sys.readFile`).
    pub label: &'static str,
    /// LSP completion kind (Function, Module, …).
    pub kind: CompletionItemKind,
    /// Short signature shown next to the label (e.g. `(p: string) -> string`).
    pub detail: &'static str,
    /// Markdown documentation shown in the hover / detail panel.
    pub documentation: &'static str,
}

// ── Keywords + primitive type names ────────────────────────────────────

pub static KEYWORDS: &[Builtin] = &[
    Builtin {
        label: "let",
        kind: CompletionItemKind::KEYWORD,
        detail: "declare a mutable binding",
        documentation: "`let x: type = expr;` binds `x` to the value of `expr`. The binding is mutable; subsequent `x = …` reassigns it.",
    },
    Builtin {
        label: "const",
        kind: CompletionItemKind::KEYWORD,
        detail: "declare an immutable binding",
        documentation: "`const X: type = expr;` binds `X` to the value of `expr`. Reassignment is a compile error.",
    },
    Builtin {
        label: "function",
        kind: CompletionItemKind::KEYWORD,
        detail: "define a named function",
        documentation: "`function name(p1: T1, p2: T2): R { … }` declares a function. Parameters and the return type can be annotated.",
    },
    Builtin {
        label: "return",
        kind: CompletionItemKind::KEYWORD,
        detail: "exit the current function with a value",
        documentation: "`return expr;` exits the enclosing function, returning `expr`. Omit the expression to return `()`.",
    },
    Builtin {
        label: "if",
        kind: CompletionItemKind::KEYWORD,
        detail: "conditional branch",
        documentation: "`if (cond) { … } else { … }` runs the `then` block when `cond` is truthy, otherwise the `else` block (if any).",
    },
    Builtin {
        label: "else",
        kind: CompletionItemKind::KEYWORD,
        detail: "fallback branch of an `if`",
        documentation: "Pairs with `if`. Run when the `if` condition is falsy.",
    },
    Builtin {
        label: "while",
        kind: CompletionItemKind::KEYWORD,
        detail: "loop while a condition holds",
        documentation: "`while (cond) { … }` repeats the body as long as `cond` is truthy. Use `break` to exit early.",
    },
    Builtin {
        label: "for",
        kind: CompletionItemKind::KEYWORD,
        detail: "C-style or range for loop",
        documentation: "`for (init; cond; update) { … }` is the C-style form. Use `for (x of array) { … }` to iterate.",
    },
    Builtin {
        label: "of",
        kind: CompletionItemKind::KEYWORD,
        detail: "iterate the elements of an array",
        documentation: "`for (x of arr) { … }` iterates each element of `arr`.",
    },
    Builtin {
        label: "break",
        kind: CompletionItemKind::KEYWORD,
        detail: "exit the innermost loop",
        documentation: "Jumps out of the nearest enclosing `while` or `for`.",
    },
    Builtin {
        label: "continue",
        kind: CompletionItemKind::KEYWORD,
        detail: "skip to the next iteration",
        documentation: "Skips the rest of the current loop body and starts the next iteration.",
    },
    Builtin {
        label: "true",
        kind: CompletionItemKind::KEYWORD,
        detail: "boolean literal `true`",
        documentation: "The `boolean` literal `true`.",
    },
    Builtin {
        label: "false",
        kind: CompletionItemKind::KEYWORD,
        detail: "boolean literal `false`",
        documentation: "The `boolean` literal `false`.",
    },
    Builtin {
        label: "string",
        kind: CompletionItemKind::KEYWORD,
        detail: "primitive type: UTF-8 string",
        documentation: "Maps to `String` in the emitted Rust.",
    },
    Builtin {
        label: "number",
        kind: CompletionItemKind::KEYWORD,
        detail: "primitive type: 64-bit float",
        documentation: "Maps to `f64` in the emitted Rust.",
    },
    Builtin {
        label: "boolean",
        kind: CompletionItemKind::KEYWORD,
        detail: "primitive type: bool",
        documentation: "Maps to `bool` in the emitted Rust.",
    },
    Builtin {
        label: "void",
        kind: CompletionItemKind::KEYWORD,
        detail: "primitive type: unit `()`",
        documentation: "Maps to `()` in the emitted Rust.",
    },
    Builtin {
        label: "import",
        kind: CompletionItemKind::KEYWORD,
        detail: "import a module as a namespace",
        documentation: "`import utils;` imports the module `utils.tsr` as a namespace.\n\n`import utils as u;` imports with an alias.\n\n`import os.path;` imports a nested module.",
    },
    Builtin {
        label: "from",
        kind: CompletionItemKind::KEYWORD,
        detail: "import specific names from a module",
        documentation: "`from utils import add, sub;` imports specific names.\n\n`from utils import add as suma;` imports with aliases.\n\n`from utils import *;` imports everything.",
    },
    Builtin {
        label: "export",
        kind: CompletionItemKind::KEYWORD,
        detail: "export a declaration from the current module",
        documentation: "`export function f() {}` / `export const X = ...;` exports inline declarations.\n\n`export { a, b as c };` re-exports existing names.\n\n`export default ...` exports a default value.",
    },
    Builtin {
        label: "as",
        kind: CompletionItemKind::KEYWORD,
        detail: "alias a binding",
        documentation: "Used in `import utils as u;` and `from utils import add as suma;` to rename bindings.",
    },
    Builtin {
        label: "default",
        kind: CompletionItemKind::KEYWORD,
        detail: "import or export a default binding",
        documentation: "`export default function() {}` / `export default expr;` to export a default.\n\n`from utils import default as calc;` to import a default export.",
    },
];

// ── Top-level builtins (print, input, sys.X where X is a top-level method) ─

pub static TOP_LEVEL_BUILTINS: &[Builtin] = &[
    Builtin {
        label: "sys",
        kind: CompletionItemKind::MODULE,
        detail: "system namespace",
        documentation: "`sys.*` — system-level builtins (filesystem, OS, env, processes, …).\n\nUse `sys.X(...)` for top-level methods or `sys.ns.method(...)` for namespaced methods.",
    },
    Builtin {
        label: "print",
        kind: CompletionItemKind::FUNCTION,
        detail: "(...values: any[]) -> void",
        documentation: "Print each argument to stdout, separated by a space, terminated by a newline.",
    },
    Builtin {
        label: "input",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> string",
        documentation: "Reads a line from stdin and returns it (without the trailing newline).",
    },
    Builtin {
        label: "parseFloat",
        kind: CompletionItemKind::FUNCTION,
        detail: "(s: string) -> number",
        documentation: "Converts a string to a floating-point number. Returns `NaN` if the string is not a valid number.",
    },
    Builtin {
        label: "isNaN",
        kind: CompletionItemKind::FUNCTION,
        detail: "(n: number) -> boolean",
        documentation: "Returns `true` if the value is `NaN` (not a number). Use to validate results from `parseFloat`.",
    },
    Builtin {
        label: "str",
        kind: CompletionItemKind::FUNCTION,
        detail: "(value: any) -> string",
        documentation: "Converts any value to its string representation. Use for explicit string concatenation.",
    },
    Builtin {
        label: "typeof",
        kind: CompletionItemKind::KEYWORD,
        detail: "typeof expr -> string",
        documentation: "Returns the type of an expression as a string: `\"number\"`, `\"string\"`, `\"boolean\"`, `\"array\"`, `\"object\"`, or `\"void\"`.",
    },
    // sys.fs
    Builtin {
        label: "sys.readFile",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> string",
        documentation: "`sys.readFile(p)` reads `p` to a UTF-8 `String`. Panics on I/O error.",
    },
    Builtin {
        label: "sys.writeFile",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string, content: string) -> void",
        documentation: "`sys.writeFile(p, c)` overwrites `p` with `c`.",
    },
    Builtin {
        label: "sys.readBytes",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> bytes",
        documentation: "`sys.readBytes(p)` reads `p` to a `Vec<u8>`. Arcis has no `bytes` literal yet, so this is mostly useful for round-trip.",
    },
    Builtin {
        label: "sys.writeBytes",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string, bytes: bytes) -> void",
        documentation: "`sys.writeBytes(p, b)` overwrites `p` with the bytes `b`.",
    },
    Builtin {
        label: "sys.appendFile",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string, text: string) -> void",
        documentation: "`sys.appendFile(p, t)` opens `p` in append mode (creating it if missing) and writes `t`.",
    },
    Builtin {
        label: "sys.createFile",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> void",
        documentation: "`sys.createFile(p)` creates an empty file at `p`.",
    },
    Builtin {
        label: "sys.deleteFile",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> void",
        documentation: "`sys.deleteFile(p)` removes the file at `p`. Panics if `p` is not a file.",
    },
    Builtin {
        label: "sys.deleteDir",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> void",
        documentation: "`sys.deleteDir(p)` removes the empty directory at `p`.",
    },
    Builtin {
        label: "sys.deleteDirAll",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> void",
        documentation: "`sys.deleteDirAll(p)` removes `p` recursively (files and sub-directories).",
    },
    Builtin {
        label: "sys.mkdir",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> void",
        documentation: "`sys.mkdir(p)` creates the directory `p`. Panics if `p` already exists.",
    },
    Builtin {
        label: "sys.listDir",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> string[]",
        documentation: "`sys.listDir(p)` returns the entry names in `p` as `string[]`.",
    },
    Builtin {
        label: "sys.copy",
        kind: CompletionItemKind::FUNCTION,
        detail: "(src: string, dst: string) -> void",
        documentation: "`sys.copy(s, d)` copies file `s` to `d`.",
    },
    Builtin {
        label: "sys.move",
        kind: CompletionItemKind::FUNCTION,
        detail: "(src: string, dst: string) -> void",
        documentation: "`sys.move(s, d)` renames file `s` to `d`.",
    },
    Builtin {
        label: "sys.rename",
        kind: CompletionItemKind::FUNCTION,
        detail: "(old: string, new: string) -> void",
        documentation: "`sys.rename(o, n)` is an alias of `sys.move`.",
    },
    // sys.path
    Builtin {
        label: "sys.exists",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> boolean",
        documentation: "`sys.exists(p)` returns `true` if `p` exists.",
    },
    Builtin {
        label: "sys.isFile",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> boolean",
        documentation: "`sys.isFile(p)` returns `true` if `p` is a regular file.",
    },
    Builtin {
        label: "sys.isDir",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> boolean",
        documentation: "`sys.isDir(p)` returns `true` if `p` is a directory.",
    },
    Builtin {
        label: "sys.fileSize",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> number",
        documentation: "`sys.fileSize(p)` returns the size of `p` in bytes.",
    },
    Builtin {
        label: "sys.fileInfo",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> string",
        documentation: "`sys.fileInfo(p)` returns a summary `size=…;is_file=…;is_dir=…;modified_secs=…`.",
    },
    Builtin {
        label: "sys.absolute",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> string",
        documentation: "`sys.absolute(p)` returns the absolute (canonical) form of `p`.",
    },
    Builtin {
        label: "sys.relative",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> string",
        documentation: "`sys.relative(p)` returns `p` relative to the current working directory.",
    },
    Builtin {
        label: "sys.createSymlink",
        kind: CompletionItemKind::FUNCTION,
        detail: "(target: string, linkPath: string) -> void",
        documentation: "`sys.createSymlink(t, l)` creates a symlink at `l` pointing to `t`.",
    },
    Builtin {
        label: "sys.readLink",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> string",
        documentation: "`sys.readLink(p)` returns the target of the symlink at `p`.",
    },
    // sys.* (process env)
    Builtin {
        label: "sys.currentDir",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> string",
        documentation: "`sys.currentDir()` returns the current working directory.",
    },
    Builtin {
        label: "sys.changeDir",
        kind: CompletionItemKind::FUNCTION,
        detail: "(path: string) -> void",
        documentation: "`sys.changeDir(p)` changes the process working directory to `p`.",
    },
    Builtin {
        label: "sys.tempDir",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> string",
        documentation: "`sys.tempDir()` returns the OS temporary directory (e.g. `/tmp`).",
    },
    Builtin {
        label: "sys.homeDir",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> string",
        documentation: "`sys.homeDir()` returns the current user's home directory.",
    },
    Builtin {
        label: "sys.executablePath",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> string",
        documentation: "`sys.executablePath()` returns the absolute path of the running binary.",
    },
    // sys.process
    Builtin {
        label: "sys.process",
        kind: CompletionItemKind::FUNCTION,
        detail: "(cmd: string, args: string[]) -> ArcisProcess",
        documentation: "`sys.process(cmd, args)` runs `cmd` with the given arguments and returns an `ArcisProcess { stdout, stderr, exitCode }` struct.",
    },
    Builtin {
        label: "sys.exec",
        kind: CompletionItemKind::FUNCTION,
        detail: "(cmd: string, args?: string[]) -> string",
        documentation: "`sys.exec(cmd, args?)` runs `cmd` and returns stdout as a lossy string.",
    },
    Builtin {
        label: "sys.spawn",
        kind: CompletionItemKind::FUNCTION,
        detail: "(cmd: string, args?: string[]) -> number",
        documentation: "`sys.spawn(cmd, args?)` runs `cmd` detached and returns its PID.",
    },
    Builtin {
        label: "sys.kill",
        kind: CompletionItemKind::FUNCTION,
        detail: "(pid: number) -> void",
        documentation: "`sys.kill(pid)` sends SIGTERM to `pid`.",
    },
    Builtin {
        label: "sys.currentPid",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> number",
        documentation: "`sys.currentPid()` returns the current process ID.",
    },
    Builtin {
        label: "sys.parentPid",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> number",
        documentation: "`sys.parentPid()` returns the parent process ID.",
    },
    Builtin {
        label: "sys.processes",
        kind: CompletionItemKind::FUNCTION,
        detail: "() -> string[]",
        documentation: "`sys.processes()` returns `pid=<n>;name=<s>` per running process.",
    },
    // sys.args (member)
    Builtin {
        label: "sys.args",
        kind: CompletionItemKind::PROPERTY,
        detail: "string[]",
        documentation: "Program arguments (the contents of `std::env::args()`). Member access — no parentheses.",
    },
];

// ── Sub-namespace accessors (offered after `sys.`) ────────────────────

pub static SYS_NAMESPACES: &[Builtin] = &[
    Builtin {
        label: "env",
        kind: CompletionItemKind::MODULE,
        detail: "environment variables (get/set/delete/all)",
        documentation: "`sys.env.*` — environment-variable accessors.",
    },
    Builtin {
        label: "os",
        kind: CompletionItemKind::MODULE,
        detail: "operating-system info",
        documentation: "`sys.os.*` — OS-level info (name, arch, hostname, …).",
    },
    Builtin {
        label: "memory",
        kind: CompletionItemKind::MODULE,
        detail: "RAM info (Linux-first via /proc/meminfo)",
        documentation: "`sys.memory.*` — system memory. Returns 0 on non-Linux.",
    },
    Builtin {
        label: "cpu",
        kind: CompletionItemKind::MODULE,
        detail: "CPU info (Linux-first via /proc/cpuinfo and /proc/stat)",
        documentation: "`sys.cpu.*` — CPU model, brand, frequency, usage, core count.",
    },
    Builtin {
        label: "gpu",
        kind: CompletionItemKind::MODULE,
        detail: "GPU info (Linux-first via lspci / nvidia-smi)",
        documentation: "`sys.gpu.*` — enumerate GPUs and query memory.",
    },
    Builtin {
        label: "disk",
        kind: CompletionItemKind::MODULE,
        detail: "disk info (cross-platform via df)",
        documentation: "`sys.disk.*` — filesystem space.",
    },
    Builtin {
        label: "net",
        kind: CompletionItemKind::MODULE,
        detail: "network info (hostname, interfaces, ip, publicIp, online)",
        documentation: "`sys.net.*` — host networking.",
    },
];

// ── Per-namespace method tables ─────────────────────────────────────────

pub static NS_METHODS: &[(&str, &[Builtin])] = &[
    (
        "env",
        &[
            Builtin { label: "get",    kind: CompletionItemKind::METHOD, detail: "(name: string) -> string",             documentation: "`sys.env.get(name)` returns the value of env var `name`, or `\"\"` if not set." },
            Builtin { label: "set",    kind: CompletionItemKind::METHOD, detail: "(name: string, value: string) -> void", documentation: "`sys.env.set(name, value)` sets env var `name`." },
            Builtin { label: "delete", kind: CompletionItemKind::METHOD, detail: "(name: string) -> void",                documentation: "`sys.env.delete(name)` unsets env var `name`." },
            Builtin { label: "all",    kind: CompletionItemKind::METHOD, detail: "() -> string[]",                        documentation: "`sys.env.all()` returns every env var as `name=value` strings." },
        ],
    ),
    (
        "os",
        &[
            Builtin { label: "name",     kind: CompletionItemKind::METHOD, detail: "() -> string", documentation: "`sys.os.name()` — `\"linux\"` / `\"macos\"` / `\"windows\"`." },
            Builtin { label: "version",  kind: CompletionItemKind::METHOD, detail: "() -> string", documentation: "`sys.os.version()` runs `uname -r` (or `ver` on Windows)." },
            Builtin { label: "arch",     kind: CompletionItemKind::METHOD, detail: "() -> string", documentation: "`sys.os.arch()` returns `std::env::consts::ARCH`." },
            Builtin { label: "hostname", kind: CompletionItemKind::METHOD, detail: "() -> string", documentation: "`sys.os.hostname()` runs the `hostname` command." },
            Builtin { label: "username", kind: CompletionItemKind::METHOD, detail: "() -> string", documentation: "`sys.os.username()` returns `$USER` (or `$USERNAME` on Windows)." },
            Builtin { label: "uptime",   kind: CompletionItemKind::METHOD, detail: "() -> number", documentation: "`sys.os.uptime()` parses `/proc/uptime` on Linux." },
            Builtin { label: "locale",   kind: CompletionItemKind::METHOD, detail: "() -> string", documentation: "`sys.os.locale()` returns `$LC_ALL` or `$LANG`." },
            Builtin { label: "cpuCount", kind: CompletionItemKind::METHOD, detail: "() -> number", documentation: "`sys.os.cpuCount()` returns the available parallelism." },
        ],
    ),
    (
        "memory",
        &[
            Builtin { label: "total",     kind: CompletionItemKind::METHOD, detail: "() -> number", documentation: "`sys.memory.total()` — total RAM in bytes (Linux-first)." },
            Builtin { label: "free",      kind: CompletionItemKind::METHOD, detail: "() -> number", documentation: "`sys.memory.free()` — free RAM in bytes." },
            Builtin { label: "used",      kind: CompletionItemKind::METHOD, detail: "() -> number", documentation: "`sys.memory.used()` — total - free." },
            Builtin { label: "available", kind: CompletionItemKind::METHOD, detail: "() -> number", documentation: "`sys.memory.available()` — kernel's `MemAvailable` estimate." },
        ],
    ),
    (
        "cpu",
        &[
            Builtin { label: "model",     kind: CompletionItemKind::METHOD, detail: "() -> string",  documentation: "`sys.cpu.model()` — `model name` from `/proc/cpuinfo`." },
            Builtin { label: "brand",     kind: CompletionItemKind::METHOD, detail: "() -> string",  documentation: "`sys.cpu.brand()` — `vendor_id` (e.g. `GenuineIntel`)." },
            Builtin { label: "frequency", kind: CompletionItemKind::METHOD, detail: "() -> number",  documentation: "`sys.cpu.frequency()` — MHz from `/proc/cpuinfo`." },
            Builtin { label: "usage",     kind: CompletionItemKind::METHOD, detail: "() -> number",  documentation: "`sys.cpu.usage()` — samples `/proc/stat` twice with a 100ms sleep and returns busy %." },
            Builtin { label: "cores",     kind: CompletionItemKind::METHOD, detail: "() -> number",  documentation: "`sys.cpu.cores()` — available parallelism." },
        ],
    ),
    (
        "gpu",
        &[
            Builtin { label: "list",   kind: CompletionItemKind::METHOD, detail: "() -> string[]", documentation: "`sys.gpu.list()` parses `lspci -vmm` for VGA / 3D / Display." },
            Builtin { label: "name",   kind: CompletionItemKind::METHOD, detail: "() -> string",   documentation: "`sys.gpu.name()` — first GPU device name." },
            Builtin { label: "vendor", kind: CompletionItemKind::METHOD, detail: "() -> string",   documentation: "`sys.gpu.vendor()` — first GPU vendor." },
            Builtin { label: "memory", kind: CompletionItemKind::METHOD, detail: "() -> number",   documentation: "`sys.gpu.memory()` — first NVIDIA GPU's total memory in bytes (via `nvidia-smi`)." },
        ],
    ),
    (
        "disk",
        &[
            Builtin { label: "list",  kind: CompletionItemKind::METHOD, detail: "() -> string[]",      documentation: "`sys.disk.list()` parses `df -B1 -P` into `mount=…;size=…;used=…;avail=…` rows." },
            Builtin { label: "free",  kind: CompletionItemKind::METHOD, detail: "(path: string) -> number", documentation: "`sys.disk.free(p)` — available bytes on the filesystem containing `p`." },
            Builtin { label: "used",  kind: CompletionItemKind::METHOD, detail: "(path: string) -> number", documentation: "`sys.disk.used(p)` — used bytes on the filesystem containing `p`." },
            Builtin { label: "total", kind: CompletionItemKind::METHOD, detail: "(path: string) -> number", documentation: "`sys.disk.total(p)` — total bytes on the filesystem containing `p`." },
        ],
    ),
    (
        "net",
        &[
            Builtin { label: "hostname",  kind: CompletionItemKind::METHOD, detail: "() -> string",  documentation: "`sys.net.hostname()` runs the `hostname` command." },
            Builtin { label: "interfaces",kind: CompletionItemKind::METHOD, detail: "() -> string[]",documentation: "`sys.net.interfaces()` parses `ip -o -4 addr show` into `iface=…;ip=…` rows." },
            Builtin { label: "ip",        kind: CompletionItemKind::METHOD, detail: "() -> string",  documentation: "`sys.net.ip()` — first token of `hostname -I`." },
            Builtin { label: "publicIp",  kind: CompletionItemKind::METHOD, detail: "() -> string",  documentation: "`sys.net.publicIp()` — `curl -s --max-time 5 https://ifconfig.me`." },
            Builtin { label: "online",    kind: CompletionItemKind::METHOD, detail: "() -> boolean", documentation: "`sys.net.online()` — `ping -c 1 -W 3 1.1.1.1` exit-success." },
        ],
    ),
    (
        "process",
        &[
            Builtin { label: "process",   kind: CompletionItemKind::METHOD, detail: "(cmd: string, args: string[]) -> ArcisProcess", documentation: "`sys.process.process(cmd, args)` — duplicate top-level path under the namespace for symmetry." },
            Builtin { label: "exec",      kind: CompletionItemKind::METHOD, detail: "(cmd: string, args?: string[]) -> string",       documentation: "Same as top-level `sys.exec`." },
            Builtin { label: "spawn",     kind: CompletionItemKind::METHOD, detail: "(cmd: string, args?: string[]) -> number",       documentation: "Same as top-level `sys.spawn`." },
            Builtin { label: "kill",      kind: CompletionItemKind::METHOD, detail: "(pid: number) -> void",                          documentation: "Same as top-level `sys.kill`." },
            Builtin { label: "currentPid",kind: CompletionItemKind::METHOD, detail: "() -> number",                                   documentation: "Same as top-level `sys.currentPid`." },
            Builtin { label: "parentPid", kind: CompletionItemKind::METHOD, detail: "() -> number",                                   documentation: "Same as top-level `sys.parentPid`." },
            Builtin { label: "processes", kind: CompletionItemKind::METHOD, detail: "() -> string[]",                                 documentation: "Same as top-level `sys.processes`." },
        ],
    ),
];

// ── Method chains on array / string receivers ──────────────────────────

pub static ARRAY_METHODS: &[Builtin] = &[
    Builtin { label: "find",    kind: CompletionItemKind::METHOD, detail: "(p: (T) => boolean) -> T | undefined",  documentation: "Returns the first element matching the predicate." },
    Builtin { label: "filter",  kind: CompletionItemKind::METHOD, detail: "(p: (T) => boolean) -> T[]",            documentation: "Returns the elements matching the predicate." },
    Builtin { label: "map",     kind: CompletionItemKind::METHOD, detail: "<U>(f: (T) => U) -> U[]",                documentation: "Returns the array of `f` applied to each element." },
    Builtin { label: "reduce",  kind: CompletionItemKind::METHOD, detail: "<U>(f: (acc, T) => U, init: U) -> U",     documentation: "Left-fold; `init` is the initial accumulator." },
    Builtin { label: "pop",     kind: CompletionItemKind::METHOD, detail: "() -> T",                                documentation: "Removes and returns the last element." },
    Builtin { label: "push",    kind: CompletionItemKind::METHOD, detail: "(item: T) -> void",                      documentation: "Appends `item`." },
    Builtin { label: "unshift", kind: CompletionItemKind::METHOD, detail: "(item: T) -> void",                      documentation: "Prepends `item`." },
    Builtin { label: "length",  kind: CompletionItemKind::PROPERTY, detail: "number",                                documentation: "Element count." },
];

pub static STRING_METHODS: &[Builtin] = &[
    Builtin { label: "toUpperCase", kind: CompletionItemKind::METHOD, detail: "() -> string",     documentation: "Uppercase copy." },
    Builtin { label: "toLowerCase", kind: CompletionItemKind::METHOD, detail: "() -> string",     documentation: "Lowercase copy." },
    Builtin { label: "trim",        kind: CompletionItemKind::METHOD, detail: "() -> string",     documentation: "Strips leading and trailing whitespace." },
    Builtin { label: "substring",   kind: CompletionItemKind::METHOD, detail: "(start: number, end?: number) -> string", documentation: "Sub-range slice." },
    Builtin { label: "indexOf",     kind: CompletionItemKind::METHOD, detail: "(s: string) -> number", documentation: "Index of first occurrence, or -1." },
    Builtin { label: "includes",    kind: CompletionItemKind::METHOD, detail: "(s: string) -> boolean", documentation: "True if `s` is a substring." },
    Builtin { label: "charAt",      kind: CompletionItemKind::METHOD, detail: "(i: number) -> string",   documentation: "Character at `i` (as a 1-char string)." },
    Builtin { label: "length",      kind: CompletionItemKind::PROPERTY, detail: "number", documentation: "Codepoint count." },
];

// ── Lookup helpers used by completion / hover ──────────────────────────

/// Look up the methods for a given namespace name (case-sensitive).
pub fn ns_methods(ns: &str) -> &'static [Builtin] {
    NS_METHODS
        .iter()
        .find(|(name, _)| *name == ns)
        .map(|(_, m)| *m)
        .unwrap_or(&[])
}

/// Look up a single builtin by its full label (e.g. `sys.readFile`).
/// Used by hover. Returns `None` if not in the table.
pub fn find_by_label(label: &str) -> Option<Builtin> {
    if let Some(b) = KEYWORDS.iter().find(|b| b.label == label) {
        return Some(*b);
    }
    if let Some(b) = TOP_LEVEL_BUILTINS.iter().find(|b| b.label == label) {
        return Some(*b);
    }
    if let Some(b) = SYS_NAMESPACES.iter().find(|b| b.label == label) {
        return Some(*b);
    }
    for (_, methods) in NS_METHODS {
        if let Some(b) = methods.iter().find(|b| b.label == label) {
            return Some(*b);
        }
    }
    if let Some(b) = ARRAY_METHODS.iter().find(|b| b.label == label) {
        return Some(*b);
    }
    if let Some(b) = STRING_METHODS.iter().find(|b| b.label == label) {
        return Some(*b);
    }
    None
}
