# `01-hello/` — the smallest possible Arcis program

A single line of source code that prints a greeting. The smallest
runnable Arcis program.

## Run it

```bash
arcis run examples/01-hello/hello.tsr
```

## Source

```ts
print("Hello, World!");
```

## Expected output

```
Hello, World!
```

## How it works

`print` is a builtin function that translates directly to Rust's
`println!("{}", arg)`. Anything passed to `print` must implement the
`Display` trait, which `String`, `f64`, and `bool` all do. Object
types print via their `#[derive(Debug)]` representation (we use
`Clone` for now; full `Debug` support is on the roadmap).

A bare `print("Hello, World!");` is the minimum: one statement, no
imports, no functions, no entry-point boilerplate.

## Notes

- `print` is **not** the same as `console.log` from JavaScript — there
  is no `console.log` here. Arcis has exactly one I/O builtin for
  output, and it's `print`.
- A trailing newline is always appended (matching Rust's `println!`).
  Use `print` exactly as `println!` would behave; there is no `print`
  without newline in v1.
- The `.tsr` extension on the file is what tells the linker this is
  an Arcis source file. Internally we derive a Rust module id from
  the file name (`hello` here).

## Next steps

- See [`examples/02-basics/`](../02-basics/) for variables and types.
- See [`examples/03-functions/`](../03-functions/) for declaring your
  own helpers.
- See [`examples/05-files/`](../05-files/) for reading and writing
  the filesystem.