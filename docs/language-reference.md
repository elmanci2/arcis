# Language reference

A formal-ish description of the Arcis language as it stands in version 0.1.0.

## Source files

Arcis source files use the `.tsr` extension. A project is a directory
containing exactly one `main.tsr` (entry point) plus zero or more other
`.tsr` files (modules). Modules support both ES/TS-style imports
(`import { a } from "mod";`) and Python-style imports
(`from mod import a;`) — see the [Modules](../README.md#modules) section
in the README, and the grammar below.

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

type_param_list = "<" IDENT ( "," IDENT )* ">" ;                 -- generics (Rust backend only)
type_alias_stmt = "type" IDENT type_param_list? "=" type ";" ;
interface_stmt  = "interface" IDENT type_param_list? ( "extends" IDENT ( "," IDENT )* )?
                   "{" ( IDENT "?"? ":" type ( ( "," | ";" ) IDENT "?"? ":" type )* ( "," | ";" )? )? "}" ;
enum_stmt    = "enum" IDENT "{" ( IDENT ( "=" NUMBER )? ( "," IDENT ( "=" NUMBER )? )* ","? )? "}" ;

switch_stmt  = "switch" "(" expression ")" "{" switch_case* "}" ;
switch_case  = ( "case" expression ":" )+ statement*
             | "default" ":" statement* ;
try_stmt     = "try" "{" statement* "}" "catch" ( "(" IDENT ")" )? "{" statement* "}" ;
throw_stmt   = "throw" expression ";" ;

let_stmt     = "let"  IDENT ( ":" type )? "=" expression ";" ;
const_stmt   = "const" IDENT ( ":" type )? "=" expression ";" ;

function_stmt = "function" IDENT type_param_list?
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

import_stmt  = "import" module_path ( "as" IDENT )? ";"                        -- namespace import (Python style)
             | "from" module_spec "import" ( "*" | import_name_list ) ";"      -- named import (Python style)
             | "import" "{" import_name_list? "}" "from" module_spec ";"       -- named import (ES style)
             | "import" IDENT ( "," "{" import_name_list? "}" )? "from" module_spec ";"  -- default (+named) import (ES style)
             | "import" "*" "as" IDENT "from" module_spec ";" ;                -- namespace import (ES style)
module_spec  = module_path | STRING ;               -- "utils", "./utils", "dir/utils", "crate:serde"
module_path  = ( IDENT | "crate" ":" IDENT ) ( "." IDENT )* ;
import_name_list = import_name ( "," import_name )* ;
import_name  = ( IDENT | "default" ) ( "as" IDENT )? ;

export_stmt  = "export" "default" ( "function" ... | expression ) ";"
             | "export" "{" export_spec_list? "}" ";"
             | "export" function_stmt
             | "export" const_stmt
             | "export" let_stmt
             | "export" type_alias_stmt
             | "export" interface_stmt
             | "export" enum_stmt ;

expression   = nullish ;
nullish      = or ( "??" or )* ;                               -- loosest-binding; see "Null safety"
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
             | IDENT type_args? ( "(" args? ")" )?          -- identifier / call, optional turbofish
             | IDENT ( "::" IDENT )+ ( "(" args? ")" )?      -- static path / call
             | "(" expression ")"
             | arrow_fn
             | "[" ( array_element ( "," array_element )* )? "]"
             | "{" ( object_field ( "," object_field )* )? "}" ;

arrow_fn     = "(" ( IDENT ":" type ( "," IDENT ":" type )* )? ")" ( ":" type )?
               "=>" ( expression | "{" statement* "}" ) ;
array_element = expression | "..." expression ;
object_field  = IDENT ":" expression | "..." expression ;
-- Explicit turbofish type args at a call site: `identity<number>(5)`. Only
-- valid on a bare-identifier call (not a chained `.method()` or `a::b()`
-- path call). Needed when a type param appears only in the return type and
-- can't be inferred from the arguments. `<` here is ambiguous with the `<`
-- comparison operator; the parser resolves it by attempting this production
-- and rolling back to an ordinary comparison on any mismatch — see
-- `crates/arcis-parser/src/expr.rs`'s `try_parse_call_type_args`.
type_args    = "<" type ( "," type )* ">" ;

type         = union_type ;
union_type   = intersection_type ( "|" intersection_type )* ;
intersection_type = postfix_type ( "&" postfix_type )* ;
postfix_type = primary_type "[]"* "?"? ;                      -- trailing `?` makes it optional
primary_type = "string" | "number" | "boolean" | "void"
             | "null" | "undefined"
             | STRING | NUMBER | "true" | "false"                -- literal types
             | IDENT ( "<" type ( "," type )* ">" )?              -- named (alias / interface), optionally parameterized
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
| Arrays + object literals | ✔ — declared type optional, inferred from the literal when omitted |
| Type inference       | ✔ `let x = 5;` → `number`, `let o = { a: 1 };` → inline object type, `for (let p of arr)` → element type, function return types inferred from `return` statements, inline arrow callback params/returns inferred from the receiver. Explicit annotations always win. Runs before codegen on BOTH backends and inside the LSP (hover shows inferred types). |
| Array methods        | `find`, `filter`, `map`, `reduce`, `pop`, `push`, `unshift` |
| String methods       | `toUpperCase`, `toLowerCase`, `trim`, `substring`, `indexOf`, `includes`, `charAt`, `.length` |
| Modules              | ✔ ES/TS style (`import { a, b as c } from "mod"`, `import def from "mod"`, `import * as ns from "mod"`) AND Python style (`import mod as alias`, `from mod import a, b as c`, `from mod import default as d`, `from mod import *`); `export` is ES-style, including `export type`/`export interface`/`export enum` — all importable cross-module (both backends) |
| External crates      | ✔ `from crate:<name> import X;` / `import { X } from "crate:<name>";` → `use <name>::X;` (Rust backend only; the crate must be visible to `rustc`/Cargo) |
| Union types           | ✔ `A \| B` — erased to the first member's Rust type at codegen (see below) |
| Intersection types     | ✔ `A & B` — same erasure as unions |
| Literal types          | ✔ `"left" \| "right" \| "center"`, `-1 \| 0 \| 1`, `true \| false` |
| Type aliases           | ✔ `type X = ...;` (including `export type`) — resolved away before codegen |
| Interfaces             | ✔ `interface X { ... }`, `extends` (multiple, merged fields) — emits a named `pub struct` |
| Optional properties    | ✔ `{ name?: string }` → `Option<T>` field, filled with `None` when omitted; `field?: T` is exactly `field: T?` — see "Null safety" |
| Optional types (`T?`) / `??` / null safety | ✔ **compile-time enforced** on both backends — every optional value must be resolved (`?? fallback`, a narrowing null-check, or `!`) before it reaches a place that expects a guaranteed value, or the build fails. See the dedicated "Null safety" section below. |
| `any`                  | — deliberately unsupported: Arcis is strongly typed and the parser rejects `any` wherever a type is expected, with a dedicated error message |
| `null` / `undefined`   | ✔ parsed as types and expressions; erase to `()` **except** where they populate a `T?` slot — see "Null safety" |
| Type assertions (`as`) | ✔ `expr as Type` — compile-time only, no runtime effect (matches TS) |
| `as const`             | ✔ parsed and erased the same way as `as Type` |
| Non-null assertion (`!`) | ✔ postfix `!` — a real, checked unwrap on a `T?` value (Rust backend: real `.unwrap()`; Cranelift: trusts the null-safety checker's static guarantee); a no-op on an already non-optional value — see "Null safety" |
| Function types          | ✔ `(a: T, b: U) => R` in type position (Rust backend only) |
| Shadowing              | ✔ `let x` re-declared in a nested block — resolved by an alpha-renaming pre-pass before validation/codegen ever see it (both backends) |
| Arrow functions         | ✔ `(x: number): number => x * 2`, block or expression body — **no variable capture**. Rust backend: lowers to a non-capturing closure. Cranelift backend: lambda-lifted into a synthetic top-level function (sound since there's no capture). Omitted return types and inline-callback parameter types are filled in by the inference pass on both backends. Usable inline as `.map`/`.filter`/`.find`/`.reduce` callbacks |
| Spread (`...`)          | ✔ in array literals (`[...a, ...b]`) and object literals (`{ ...base, field: v }`) (both backends — Cranelift via new `arcis_vec_extend`/`arcis_object_merge` runtime calls) |
| `switch`/`case`/`default` | ✔ **non-fallthrough** — each case is its own block, not a C-style fallthrough chain. Rust backend: `if`/`else if` chain (not a Rust `match`, since `number` is `f64` and Rust match patterns reject float literals). Cranelift backend: the same `==`-chain shape, built directly in Cranelift IR (both backends) |
| `try`/`catch`/`throw`   | ✔ Rust backend: `std::panic::catch_unwind`/`panic!`. Cranelift backend: `setjmp`/`longjmp`, called *directly* from the generated IR (not through a C wrapper that returns — that shape is unsound, see the caveats below). Both backends leak the thrown value's frame locals on unwind (documented, not hidden) |
| Enums                | ✔ `enum X { A, B = 5, C }` — TS *numeric enum* semantics on both backends: every variant IS a `number` (`X.B == 5` is true, a `number` field can hold it, printing shows the number). Rust backend: a unit struct with `f64` associated consts (`X::B`); Cranelift: compile-time `f64const`s. Enums declared in any module are usable from any other module. |
| Array-method callbacks (`.find`/`.filter`/`.map`/`.reduce`) | ✔ Cranelift backend builds a real loop (blocks + `brif` + indirect-free direct calls) rather than splicing the callback inline the way the Rust backend does — this was a **pre-existing gap** (0% implemented) closed in the same pass as the six features above, not a new feature of its own |
| Type checker         | — (relies on `rustc` today; annotations are not verified) |
| Classes              | —                |
| Generics             | ✔ **Rust backend only.** `function f<T>(x: T): T { ... }`, `interface Box<T> { value: T }`, `type Pair<A, B> = { first: A, second: B }`. Call sites infer type args from arguments (`f(5)`) or accept explicit turbofish (`f<number>(5)`) for params that only appear in the return type. No Arcis-side monomorphization — real Rust generics are emitted and `rustc` does the rest. The Cranelift backend rejects any generic construct with a clear error (`--backend cranelift` + generics = build error, not a silent miscompile) |
| Native JSON (`json(path)`) | ✔ **Rust backend only.** A literal path (`json("./data.json")`) is read and its shape inferred **at compile time** (nested objects/arrays/primitives), the same way an object literal's type is inferred — no hand-written interface, full field-access checking, real editor autocomplete. A non-literal path requires `json<T>(path)` naming an already-declared interface. Resolved **relative to the process's working directory**, matching `sys.readFile`'s existing runtime convention (not module-relative). Runtime deserialization uses `serde`/`serde_json`, auto-added to `Cargo.toml` (needs a Cargo.toml next to the entry file — the direct-rustc layout can't pull in external crates at all). Cranelift rejects `json(...)` with a clear error, same as generics |
| Destructuring (`const {x,y} = obj`) | — |
| Template literals (`` `${x}` ``) | — |
| Closures (capturing variables) | — (arrow functions above are non-capturing only) |
| `bigint` literals (`100n`) | —          |
| Async                | —                |
| Cranelift backend: interfaces / object shapes | ✔ interfaces and type aliases are resolved before codegen (same pass as the Rust backend); object field accesses (`p.name`, `arr[i].name`, for-of variables, nested `a.b.c`) are typed from the declared/inferred shape. Unions erase to the first member. |
| Cranelift backend: `continue`/`break` inside an `if` nested in a `for-of` loop | ✔ fixed — `continue` now routes through a dedicated increment block (it used to re-test the same element forever). |
| Cranelift backend: printing `object`-typed values | ✔ promotes to `"[object Object]"` (JS-style) instead of erroring. |

## How types flow

Arcis accepts type annotations in source and translates them to Rust
equivalents:

| Arcis    | Rust           |
|----------|----------------|
| `string` | `String`       |
| `number` | `f64`          |
| `boolean`| `bool`         |
| `void`   | `()`           |
| `null`, `undefined` (as a type on their own) | `()` |
| `T?`     | `Option<T>` — see "Null safety" below |
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
- **Type assertions** (`as Type`, `as const`) have no runtime effect,
  exactly like in TypeScript — Arcis emits just the inner expression. The
  **non-null assertion** (`!`) is the one exception: on a genuinely
  optional (`T?`) expression it compiles to a real, checked unwrap (a Rust
  `.unwrap()` — panics with a clear message if you were wrong; on
  Cranelift it trusts the sentinel is resolved, per the null-safety
  checker's guarantee) — see "Null safety" below. On an already
  non-optional expression `!` is a no-op, same as `as`.
- **`type` aliases** are resolved away entirely before codegen runs: every
  reference to an alias is replaced by its underlying type, so nothing
  downstream needs to know aliases exist. **`interface`** names are *not*
  aliases — each interface gets its own emitted struct.

## Null safety

Arcis enforces a single, simple guarantee: **a `T?` value can never reach a
place that expects a guaranteed `T` without being resolved first.** This is
a hard compile error, not a lint — on both backends, identically.

### Declaring an optional type

Three equivalent spellings, all normalized to the same internal type:

```ts
let a: number?;              // postfix `?`
let b: number | null;        // union with `null`/`undefined`
interface P { nickname?: string; } // a `?` field marker IS `nickname: string?`
```

### The three ways to resolve one

| Form | Meaning | Requirement |
|------|---------|-------------|
| `x ?? fallback` | use `x` if present, `fallback` otherwise | `fallback` must ITSELF be non-optional — "two optionals" (`x ?? y` where `y` is also `T?`) is a compile error. Chain another `?? realDefault` instead. |
| `if (x != null) { ... }` | proves `x` is present for the rest of that block | recognized for a plain identifier only (`if (obj.field != null)` isn't narrowed — bind the field to a local first: `let f = obj.field; if (f != null) { ... }`) |
| `if (x == null) { return/throw/break/continue; }` | "guard clause" — proves `x` is present for every statement AFTER the `if`, in the same function/block | the branch must unconditionally exit (no `else`, and the branch's last statement is `return`/`throw`/`break`/`continue`) |
| `x!` | explicit, deliberate assertion ("I already know this is present") | your responsibility — wrong, and the Rust backend panics with a clear message at that line; the Cranelift backend trusts you (no runtime check yet, see caveat below) |

`null`/`undefined` literals are also checked directly: assigning, passing,
or returning one where the target isn't `T?` is rejected the same way.

### What counts as "a place that expects a guaranteed value" (a *sink*)

- a `let`/`const` with an explicit non-optional type,
- a `return` inside a function whose return type is non-optional,
- an argument passed to a parameter of a known function,
- the fallback (right-hand side) of `??` itself,
- operands of arithmetic/comparison operators (other than the sanctioned
  `== null` / `!= null` idiom),
- the receiver of `.field` / `[index]`,
- the single argument to `print`/`str`.

Annotations stay optional (pun intended) everywhere the type checker's
[inference](#how-types-flow) can fill them in — `let x = maybeFind();`
infers `x` as `T?` automatically if `maybeFind` returns `T?`; you only
have to write `?` when you're annotating explicitly. Either way, the FIRST
unsafe use of that value — not the declaration — is where the checker
stops you.

### Example

See `examples/optionals/optionals.tsr` for a complete, runnable walkthrough
of all three resolution forms plus optional interface fields; the file's
trailing comment lists four one-line variants that are each, individually,
a compile error, so you can see exactly what the checker rejects and why.

### How it's implemented (for the curious)

1. **Narrowing is a rewrite, not a separate type system.** `if (x != null) { ... }` is desugared, right after type inference, into a `let` under a fresh compiler-generated name (`x!` — a real unwrap — assigned inside the guarded region), and every reference to `x` inside that region is rewritten to the fresh name. Nothing downstream (codegen, the checker itself) needs to understand control-flow narrowing as a concept.
2. **Rust backend**: `T?` is a real `Option<T>`; `??` compiles to `.clone().unwrap_or_else(|| fallback)`; `x == null` / `x != null` compile to `.is_none()` / `.is_some()`; a `return` of a definite value from a `T?`-returning function is auto-wrapped in `Some(...)`.
3. **Cranelift backend**: `T?` and `T` share the exact same physical representation (no tagged union) — "missing" is an in-band sentinel chosen so real data can't produce it: a specific quiet-NaN bit pattern for `number?`, `2` for `boolean?` (valid booleans are only 0/1), and `0`/null-handle for `string?`/`array?`/`object?` (already means "no allocation" for every handle type anyway). `??` is a genuine short-circuit branch (the fallback is only evaluated when needed), not an eagerly-evaluated `select`. **Caveat**: this is a probabilistic, not proof-carrying, guarantee — unlike Rust's `Option<T>`, there's a (astronomically small) chance real data computes to exactly the sentinel bit pattern; accepted the same way niche-value optimizations generally are. `!` has no additional runtime check on this backend (the checker's static guarantee is what you're relying on).

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

### Cranelift-backend specifics

- **Arrow functions** are lambda-lifted: each `let f = (params) => body;`
  becomes a synthetic top-level Cranelift function declared/defined through
  the exact same two-phase machinery as a real `function` (sound because
  Arcis arrows never capture). An arrow used inline as a callback
  (`arr.map(x => x * 2)`) is lambda-lifted **on the spot**, right where it's
  encountered, into a freshly declared+defined function — this works
  because Cranelift's `declare_function`/`define_function` don't require a
  single function to be "in progress" globally; a fresh `Context` can be
  built and defined while another function's `Context` is still open.
- **`try`/`catch`/`throw`** lower to `setjmp`/`longjmp` against a small
  global stack of `jmp_buf`s in the C runtion (single-threaded, so no
  locking needed). The critical implementation detail: `setjmp` is called
  **directly** by the Cranelift-generated code for the `try` statement,
  never through a C wrapper function that then returns. A `longjmp` cannot
  resume a function whose activation has already ended (C99 §7.13.2.1) —
  a first implementation attempt that called `setjmp` inside a thin
  `arcis_try_begin()` wrapper and returned its result **segfaulted
  reliably** on this target, which is how this constraint was confirmed
  empirically, not just theoretically. The fix: a normal helper
  (`arcis_try_push`) only reserves a `jmp_buf` slot and returns its
  pointer; the Cranelift IR itself then calls libc's real `setjmp` symbol
  (confirmed present as a directly-callable symbol on this target, not
  merely a header macro) so the function that "contains" the `setjmp` call
  is the Cranelift-compiled function itself, which stays on the stack for
  the whole lexical scope of the `try`. `return`/`break`/`continue` inside
  an open `try` emit matching `arcis_try_end()` calls first, to keep the
  runtime's open-try counter balanced.
- **Enums** and **`switch`** need no Cranelift-specific caveats beyond
  what's already true generally: enum values are `f64const`s (no runtime
  cost), and `switch` is the same `==`-chain shape as the Rust backend,
  built directly with `brif`/blocks instead of Rust source text.

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