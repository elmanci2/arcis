# `05-files/` — the `sys.*` filesystem builtins

Arcis ships with a small set of filesystem and `argv` builtins under
the `sys.*` namespace. They translate to the Rust standard library
and require no `import`.

## Run it

```bash
arcis run examples/05-files/files.tsr
```

The example is idempotent: if `arcis_fs_demo/` already exists from a
previous run, it skips the `mkdir` and continues.

To clean up afterwards:

```bash
rm -rf arcis_fs_demo
```

## What it covers

| Builtin                | Description                                         | Rust equivalent                       |
|------------------------|-----------------------------------------------------|---------------------------------------|
| `sys.args`             | Program arguments as `string[]`                     | `std::env::args().collect::<Vec<_>>()`|
| `sys.readFile(path)`   | Read a file's contents as a string                  | `std::fs::read_to_string(&path)?`     |
| `sys.writeFile(p, c)`  | Write a string to a file (overwrites)               | `std::fs::write(&path, &content)?`    |
| `sys.exists(path)`     | `true` if the path exists                           | `std::path::Path::new(&path).exists()`|
| `sys.listDir(path)`    | List directory entries as `string[]`                | `std::fs::read_dir(&path)` + collect  |
| `sys.mkdir(path)`      | Create a single directory                           | `std::fs::create_dir(&path)?`        |
| `sys.deleteFile(path)` | Delete a file (not a directory)                     | `std::fs::remove_file(&path)?`       |

## How it works

### Why a `sys.*` namespace?

We follow the same convention as Go (`os.Args`, `os.ReadFile`) and
Node (`process.argv`, `fs.readFileSync`). Putting filesystem
operations behind a namespace makes them visually distinct from
your own functions, and avoids polluting the global identifier
space (so you can't accidentally shadow `readFile` with a local
function).

### Why `deleteFile` and not `delete` or `rm`?

`deleteFile` mirrors `std::fs::remove_file` literally — it works on
**files only**, not directories. Removing a non-empty directory in
Rust needs `remove_dir_all`, which we don't expose yet. Until we do,
you can `rm -rf` from outside the program.

### `sys.args` is always present

Even when you don't pass arguments, `sys.args` returns a one-element
vector with the program path (matching `argv[0]` in C and the first
element of `process.argv` in Node). The example shows this — with no
arguments it still prints `arg count: 1`.

### String arguments take a `String` reference under the hood

The codegen emits `&path` (a `&String`) for every `sys.*` argument,
so the call site can pass a `let path: string = "..."` directly. You
do **not** need to write `&` yourself; the codegen handles it.

## Expected output

```
arg count: 1
first arg: bin/_05_files
created dir: arcis_fs_demo
wrote file: arcis_fs_demo/note.txt
file exists? true
missing file exists? false
--- content of arcis_fs_demo/note.txt ---
Hello from Arcis!
Line 2.

--- end content ---
entries in arcis_fs_demo:
  note.txt
deleted file: arcis_fs_demo/note.txt
file still exists? false
dir still exists? true (left in place — rm -rf to remove)
```

## Notes

- All `sys.*` calls return `Result<T, io::Error>` in Rust. The
  codegen calls `.unwrap()` on the result, so failures panic at
  runtime. Wrap your call sites in `try`/`catch` if you need to
  handle errors (not yet implemented in v1).
- External crates (HTTP, JSON, regex, …) are meant to work via a
  `Cargo.toml` next to `main.tsr` plus `from crate:<name> import ...;`,
  but that import form is currently a parse error (not yet implemented)
  — see [`examples/mods/`](../mods/) for the layout and caveat.

## Related examples

- [`examples/mods/`](../mods/) — multi-module project with a
  `Cargo.toml` for external crates.