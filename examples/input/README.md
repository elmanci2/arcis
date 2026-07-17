# `input.tsr` — the `input()` builtin

How to read one line from standard input. `input()` is a blocking
call that returns the next line of stdin (with the trailing newline
stripped) as a `string`.

## Run it

### Interactive

```bash
arcis run examples/input/input.tsr
```

The program will prompt, you type a line and press Enter, and the
greeting is printed.

### Piped input

```bash
printf "Arcis\nhello world\n" | arcis run examples/input/input.tsr
```

This is what the CLI integration tests in
`crates/arcis/tests/cli.rs` do — they pipe input so the test runs
without human interaction.

## What it covers

1. The `input()` builtin.
2. Chaining `print` and `input`.
3. String `.length` on the result.

## How it works

`input()` translates to:

```rust
{
    let mut __arcis_input = String::new();
    std::io::stdin().read_line(&mut __arcis_input).unwrap();
    __arcis_input.trim_end().to_string()
}
```

That is:

1. Allocate an empty `String`.
2. Read one line from stdin (including the trailing `\n`).
3. `.trim_end()` strips the trailing whitespace — typically just the
   `\n` but also trims trailing spaces/tabs.
4. `.to_string()` returns the trimmed result.

If you need the line **with** the trailing newline, call
`std::io::stdin().read_line(&mut x)` directly via a `crate:` import.
The `input()` builtin is the convenient case.

## Expected output

Interactive:

```
¿Cómo te llamás?
Arcis                        ← user types this
Hola, Arcis!
Contame algo:
hola mundo
Me dijiste: "hola mundo"
Tu frase tiene 10 caracteres.
```

Piped (`printf "Arcis\nhola mundo\n" | …`):

```
¿Cómo te llamás?
Hola, Arcis!

Contame algo:
Me dijiste: "hola mundo"
Tu frase tiene 10 caracteres.
```

## Notes

- `input()` is **blocking**: the program stops until a line is
  available on stdin. There is no non-blocking or timeout version
  in v1.
- `input()` takes **no arguments**. To display a prompt, call
  `print` first (this example does exactly that).
- For richer input (numbers, parsing, etc.), read the string and
  parse it manually (`parseFloat`-style conversion is not yet a
  builtin — see the roadmap).

## Related examples

- [`examples/05-files/`](../05-files/) — `sys.readFile` for
  reading from a file instead of stdin.