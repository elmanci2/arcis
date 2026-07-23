# Language reference

A formal-ish description of the Arcis language as it stands in version 0.1.0.

## Source files

Arcis source files use the `.tsr` extension. A project is a directory
containing exactly one `main.tsr` (entry point) plus zero or more other
`.tsr` files (modules). Local modules are linked by `import`; external
Rust crates by `import ... from "crate:<name>"`.

## Grammar (EBNF-ish)

```ebnf
program      = statement* ;

statement    = let_stmt
             | const_stmt
             | function_stmt
             | type_alias_stmt
             | interface_stmt
             | enum_stmt
             | if_stmt
             | while_stmt
             | for_stmt
             | switch_stmt
             | try_stmt
             | throw_stmt
             | return_stmt
             | break_stmt
             | continue_stmt
             | assign_stmt
             | expr_stmt
             | import_stmt
             | export_stmt ;

type_alias_stmt = "type" IDENT "=" type ";" ;
interface_stmt  = "interface" IDENT ( "extends" IDENT ( "," IDENT )* )?
                   "{" ( IDENT "?"? ":" type ( ( "," | ";" ) IDENT "?"? ":" type )* ( "," | ";" )? )? "}" ;
enum_stmt    = "enum" IDENT "{" ( IDENT ( "=" NUMBER )? ( "," IDENT ( "=" NUMBER )? )* ","? )? "}" ;

switch_stmt  = "switch" "(" expression ")" "{" switch_case* "}" ;
switch_case  = ( "case" expression ":" )+ statement*
             | "default" ":" statement* ;
try_stmt     = "try" "{" statement* "}" "catch" ( "(" IDENT ")" )? "{" statement* "}" ;
throw_stmt   = "throw" expression ";" ;

let_stmt     = "let"  IDENT ( ":" type )? "=" expression ";" ;
const_stmt   = "const" IDENT ( ":" type )? "=" expression ";" ;

function_stmt = "function" IDENT
                "(" params? ")"
                ( ":" type )?
                "{" statement* "}" ;

if_stmt      = "if" "(" expression ")" "{" statement* "}"
                ( "else" "{" statement* "}" )? ;
while_stmt   = "while" "(" expression ")" "{" statement* "}" ;

for_stmt     = "for" "("
                ( "let" IDENT ( ":" type )? "of" expression       -- for-of
                | "let" IDENT ( ":" type )? "=" expression ";"      -- init
                | expression? ";"                                    -- expr init
                | ";"                                                 -- empty init
                )
                expression? ";"                                       -- condition
                expression? ")"                                       -- update
                "{" statement* "}" ;

return_stmt  = "return" expression? ";" ;
break_stmt   = "break" ";" ;
continue_stmt = "continue" ";" ;

assign_stmt  = IDENT "=" expression ";"             -- plain assignment
             | IDENT "[" expression "]" "=" expression ";"   -- indexed assignment
             | expression "." IDENT "=" expression ";"       -- member assignment ;

expr_stmt    = expression ";" ;

import_stmt  = "import" ( IDENT ( "," "{" import_spec_list? "}" )?
                       | "{" import_spec_list? "}" )
                "from" STRING ";" ;

export_stmt  = "export" "default" ( "function" ... | expression ) ";"
             | "export" "{" export_spec_list? "}" ";"
             | "export" function_stmt
             | "export" const_stmt
             | "export" let_stmt
             | "export" type_alias_stmt
             | "export" interface_stmt
             | "export" enum_stmt ;

expression   = or ;
or           = and ( "||" and )* ;
and          = equality ( "&&" equality )* ;
equality     = comparison ( ( "==" | "!=" ) comparison )* ;
comparison   = additive ( ( "<" | ">" | "<=" | ">=" ) additive )* ;
additive     = multiplicative ( ( "+" | "-" ) multiplicative )* ;
multiplicative = unary ( ( "*" | "/" | "%" ) unary )* ;
unary        = ( "!" | "-" | "typeof" ) unary | postfix ;
postfix      = atom ( "." IDENT | "[" expression "]" | "(" args? ")"
                     | "!"                                       -- non-null assertion
                     | "as" ( "const" | type )                   -- type / const assertion
                     )* ;
atom         = NUMBER | STRING | "true" | "false" | "null" | "undefined"
             | IDENT ( "::" IDENT )* ( "(" args? ")" )?    -- identifier or path / call
             | "(" expression ")"
             | arrow_fn
             | "[" ( array_element ( "," array_element )* )? "]"
             | "{" ( object_field ( "," object_field )* )? "}" ;

arrow_fn     = "(" ( IDENT ":" type ( "," IDENT ":" type )* )? ")" ( ":" type )?
               "=>" ( expression | "{" statement* "}" ) ;
array_element = expression | "..." expression ;
object_field  = IDENT ":" expression | "..." expression ;

type         = union_type ;
union_type   = intersection_type ( "|" intersection_type )* ;
intersection_type = postfix_type ( "&" postfix_type )* ;
postfix_type = primary_type "[]"* ;
primary_type = "string" | "number" | "boolean" | "void" | "any"
             | "null" | "undefined"
             | STRING | NUMBER | "true" | "false"                -- literal types
             | IDENT                                              -- named (alias / interface)
             | "{" ( IDENT "?"? ":" type ( "," IDENT "?"? ":" type )* )? "}"
             | "(" ( IDENT ":" type ( "," IDENT ":" type )* )? ")" "=>" type ;
```

## Supported subset at a glance

| Feature              | Status           |
|----------------------|------------------|
| `let`, `const`       | ✔                |
| Primitives `string`/`number`/`boolean`/`void` | ✔ |
| `function` decl      | ✔                |
| `if`/`else`          | ✔                |
| `while`              | ✔                |
| `for` (C-style)      | ✔                |
| `for (x of arr)`     | ✔                |
| `break`, `continue`  | ✔                |
| `print`, `input`     | ✔ (builtins)     |
| `arcis fmt`          | ✔ Formatter: 2-space indent, semicolons, comments preserved. CLI + LSP. |
| `sys.*` (filesystem) | ✔ `readFile`/`writeFile`/`readBytes`/`writeBytes`/`appendFile`/`createFile`/`deleteFile`/`deleteDir`/`deleteDirAll`/`mkdir`/`listDir`/`copy`/`move`/`rename` |
| `sys.*` (path)       | ✔ `exists`/`isFile`/`isDir`/`fileSize`/`fileInfo`/`absolute`/`relative`/`createSymlink`/`readLink` |
| `sys.*` (proc env)   | ✔ `args`/`currentDir`/`changeDir`/`tempDir`/`homeDir`/`executablePath` |
| `sys.*` (process)    | ✔ `process`/`exec`/`spawn`/`kill`/`currentPid`/`parentPid`/`processes`. `sys.process(cmd, args)` returns the built-in `ArcisProcess { stdout, stderr, exitCode }` struct. |
| `sys.env.*`          | ✔ `get`/`set`/`delete`/`all` (environment variables). |
| `sys.os.*`           | ✔ `name`/`version`/`arch`/`hostname`/`username`/`uptime`/`locale`/`cpuCount`. Cross-platform via stdlib. |
| `sys.memory.*`       | ✔ `total`/`free`/`used`/`available` (bytes). Linux-first via `/proc/meminfo`. |
| `sys.cpu.*`          | ✔ `model`/`brand`/`frequency`/`usage`/`cores`. Linux-first via `/proc/cpuinfo`+`/proc/stat`; `usage` blocks 100ms. |
| `sys.gpu.*`          | ✔ `list`/`name`/`vendor`/`memory`. Linux-first via `lspci` and `nvidia-smi`. |
| `sys.disk.*`         | ✔ `list`/`free`/`used`/`total`. Cross-platform via `df`. |
| `sys.net.*`          | ✔ `hostname`/`interfaces`/`ip`/`publicIp`/`online`. Linux-first via `hostname`, `ip`, `curl`, `ping`. |
| Arrays + object literals | ✔ (with declared type) |
| Array methods        | `find`, `filter`, `map`, `reduce`, `pop`, `push`, `unshift` |
| String methods       | `toUpperCase`, `toLowerCase`, `trim`, `substring`, `indexOf`, `includes`, `charAt`, `.length` |
| Modules              | `import`/`export` (TS-style)  |
| External crates      | `import ... from "crate:<name>"` |
| Union types           | ✔ `A \| B` — erased to the first member's Rust type at codegen (see below) |
| Intersection types     | ✔ `A & B` — same erasure as unions |
| Literal types          | ✔ `"left" \| "right" \| "center"`, `-1 \| 0 \| 1`, `true \| false` |
| Type aliases           | ✔ `type X = ...;` (including `export type`) — resolved away before codegen |
| Interfaces             | ✔ `interface X { ... }`, `extends` (multiple, merged fields) — emits a named `pub struct` |
| Optional properties    | ✔ `{ name?: string }` → `Option<T>` field, filled with `None` when omitted |
| `any`                  | ✔ `let`/`const` bindings skip the Rust annotation (inferred from the initializer); function params/returns lower to `Box<dyn Any>` |
| `null` / `undefined`   | ✔ parsed as types and expressions; both erase to `()` (no `Option`-based runtime yet) |
| Type assertions (`as`) | ✔ `expr as Type` — compile-time only, no runtime effect (matches TS) |
| `as const`             | ✔ parsed and erased the same way as `as Type` |
| Non-null assertion (`!`) | ✔ postfix `!` — compile-time only, no runtime null check yet |
| Function types          | ✔ `(a: T, b: U) => R` in type position (Rust backend only) |
| Shadowing              | ✔ `let x` re-declared in a nested block — resolved by an alpha-renaming pre-pass before validation/codegen ever see it (Rust backend and Cranelift backend both) |
| Arrow functions         | ✔ `(x: number): number => x * 2`, block or expression body — **no variable capture** (lowers to a non-capturing Rust closure); usable inline as `.map`/`.filter`/`.find`/`.reduce` callbacks (Rust backend only) |
| Spread (`...`)          | ✔ in array literals (`[...a, ...b]`) and object literals (`{ ...base, field: v }`) (Rust backend only) |
| `switch`/`case`/`default` | ✔ **non-fallthrough** — each case is its own block, not a C-style fallthrough chain; lowers to an `if`/`else if` chain, not a Rust `match` (`number` is `f64`, and Rust match patterns reject float literals) (Rust backend only) |
| `try`/`catch`/`throw`   | ✔ lowers to `std::panic::catch_unwind`/`panic!` — see the caveats below (Rust backend only) |
| Enums                | ✔ `enum X { A, B = 5, C }` — numeric only, emits a `pub enum`; `X.A` → `X::A` (Rust backend only) |
| Type checker         | — (relies on `rustc` today; annotations are not verified) |
| Classes              | —                |
| Generics             | —                |
| Destructuring (`const {x,y} = obj`) | — |
| Template literals (`` `${x}` ``) | — |
| Closures (capturing variables) | — (arrow functions above are non-capturing only) |
| `bigint` literals (`100n`) | —          |
| Async                | —                |
| `import * as ns`     | —                |
| Cranelift backend: unions/interfaces/aliases/`any`/enums/arrows/spread/switch/try | — (Phase 1 only covers primitives, control flow, and function calls; everything above marked "Rust backend only" returns a clear error there instead of silently doing the wrong thing) |

## How types flow

Arcis accepts type annotations in source and translates them to Rust
equivalents:

| Arcis    | Rust           |
|----------|----------------|
| `string` | `String`       |
| `number` | `f64`          |
| `boolean`| `bool`         |
| `void`   | `()`           |
| `any`    | `Box<dyn std::any::Any>` (function params/returns); no annotation on `let`/`const` (inferred) |
| `null`, `undefined` | `()` |
| `T[]`    | `Vec<T>`       |
| `{ k: T, opt?: U }` | `pub struct { pub k: T, pub opt: Option<U> }` |
| `A \| B`, `A & B` | the Rust type of `A` (first member) — see "Type erasure" below |
| `"lit"`, `42`, `true` (literal types) | `String`, `f64`, `bool` respectively |
| `(a: T) => R` | `fn(T) -> R` |

Inline object types (`{ name: string, age: number }`) get a deterministic
hash-based struct name (`__Obj12ab45`), centralised in the root module's
`main.rs`, and referenced as `crate::__Obj12ab45` from non-root modules.
`interface` declarations get the same treatment but keep their own name
(e.g. `pub struct Dog { ... }`) instead of a hash.

### Type erasure

Arcis has no runtime type checker: type annotations are compile-time-only,
same as TypeScript's own erasure model. Two consequences:

- **Unions and intersections** (`A | B`, `A & B`) lower to the Rust type of
  their *first* member — there is no tagged-union runtime, so pick the
  member order to match how the value is actually produced. This is why the
  idiomatic use of a union is a same-shaped literal union (TypeScript's own
  "string enum" pattern, e.g. `"left" | "right" | "center"`), not a
  genuinely heterogeneous union like `string | number`.
- **Type assertions** (`as Type`, `as const`) and the **non-null assertion**
  (`!`) have no runtime effect, exactly like in TypeScript — Arcis emits
  just the inner expression.
- **`type` aliases** are resolved away entirely before codegen runs: every
  reference to an alias is replaced by its underlying type, so nothing
  downstream needs to know aliases exist. **`interface`** names are *not*
  aliases — each interface gets its own emitted struct.

## Shadowing, arrow functions, `switch`, and `try`/`catch` — design notes

- **Shadowing** is resolved by an alpha-renaming pass (`arcis-validation`'s
  `resolve_shadowing`) that runs immediately after linking, before
  validation or codegen see the AST: any inner declaration that reuses an
  ancestor scope's name is renamed (`x` -> `x__shadow1`) and every
  reference within its scope is rewritten to match. A genuine same-scope
  duplicate (`let x = 1; let x = 2;` with no block in between) is left
  alone so the duplicate-declaration check still reports it.
- **Arrow functions never capture variables.** `(x: number) => x * base`
  only compiles if `base` is itself a parameter of the arrow (not an outer
  variable) — Arcis lowers arrows to non-capturing Rust closures, which
  coerce to a plain `fn` pointer. Referencing an outer variable produces a
  `rustc`-level error, not an Arcis-level one (no verification upstream).
- **`switch` is non-fallthrough**: each `case`'s body is its own block,
  never falls through into the next case's body (unlike C/JS). It lowers
  to an `if`/`else if` chain comparing the discriminant with `==`, not a
  Rust `match` — Arcis `number` is `f64`, and Rust `match` patterns reject
  float literals, so an `==`-based chain was the only encoding that works
  uniformly for numbers, strings, booleans, and enum variants. A `break;`
  inside a case body is parsed and validated but is a no-op in codegen
  (each case already ends its own `if`/`else` arm).
- **`try`/`catch` uses `std::panic::catch_unwind`** wrapped in
  `AssertUnwindSafe`, which sidesteps Rust's compile-time `UnwindSafe`
  check. This is genuinely unsound if the `try` body mutates state that's
  observed after the catch — low-risk in practice for Arcis's typical
  `String`/`Vec`/primitive locals, but a real caveat, not hidden. Also: the
  `try` body is wrapped in a Rust closure, so `return`/`break`/`continue`
  inside it do not propagate to the enclosing function/loop the way they
  would in TypeScript (`rustc` rejects `break`/`continue` there outright;
  a `return` would silently only return from the closure).

## Operator semantics

| Operator | Translation                                                       |
|----------|-------------------------------------------------------------------|
| `+` (string literal operand) | `format!("{}{}", a, b)`                              |
| `+` (no string literal)      | `a + b`                                              |
| `-`, `*`, `/`, `%`, `==`, `!=`, `<`, `>`, `<=`, `>=`, `&&`, `||`, `!` | direct Rust equivalents |

## Known gaps

See the README's "Known limitations" section. Type checking is delegated
to `rustc`; if you write `let x: number = "hi";` you'll get a Rust-level
error pointing back at your `.tsr` source.