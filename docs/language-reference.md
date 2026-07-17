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
             | if_stmt
             | while_stmt
             | for_stmt
             | return_stmt
             | break_stmt
             | continue_stmt
             | assign_stmt
             | expr_stmt
             | import_stmt
             | export_stmt ;

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
             | "export" let_stmt ;

expression   = or ;
or           = and ( "||" and )* ;
and          = equality ( "&&" equality )* ;
equality     = comparison ( ( "==" | "!=" ) comparison )* ;
comparison   = additive ( ( "<" | ">" | "<=" | ">=" ) additive )* ;
additive     = multiplicative ( ( "+" | "-" ) multiplicative )* ;
multiplicative = unary ( ( "*" | "/" | "%" ) unary )* ;
unary        = ( "!" | "-" ) unary | postfix ;
postfix      = atom ( "." IDENT | "[" expression "]" | "(" args? ")" )* ;
atom         = NUMBER | STRING | "true" | "false"
             | IDENT ( "::" IDENT )* ( "(" args? ")" )?    -- identifier or path / call
             | "(" expression ")"
             | "[" ( expression ( "," expression )* )? "]"
             | "{" ( IDENT ":" expression ( "," IDENT ":" expression )* )? "}" ;

type         = "string" | "number" | "boolean" | "void" | IDENT
             | "{" ( IDENT ":" type ( "," IDENT ":" type )* )? "}"
             | <type> "[]" ;
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
| Type checker         | — (relies on `rustc` today) |
| Classes              | —                |
| Interfaces           | —                |
| Async                | —                |
| `import * as ns`     | —                |

## How types flow

Arcis accepts type annotations in source and translates them to Rust
equivalents:

| Arcis    | Rust           |
|----------|----------------|
| `string` | `String`       |
| `number` | `f64`          |
| `boolean`| `bool`         |
| `void`   | `()`           |
| `T[]`    | `Vec<T>`       |
| `{ k: T }` | `pub struct { pub k: T }` |

Inline object types (`{ name: string, age: number }`) get a deterministic
hash-based struct name (`__Obj12ab45`), centralised in the root module's
`main.rs`, and referenced as `crate::__Obj12ab45` from non-root modules.

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