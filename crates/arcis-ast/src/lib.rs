//! Arcis AST (Abstract Syntax Tree) data types.
//!
//! This crate holds the **plain data types** that represent the structure of
//! an Arcis program after parsing. Every other phase crate (lexer, parser,
//! validation, linker, codegen) is a downstream consumer of these types.
//!
//! The crate intentionally has **no dependencies** — it must be possible to
//! import the AST types from anywhere without creating a dependency cycle.
//!
//! Once the workspace is fully split, this `lib.rs` will re-export the
//! submodules (`stmt`, `expr`, `types`, `program`) so downstream code can
//! `use arcis_ast::{Expr, Stmt, Type, Program};`. For now the contents of the
//! original `src/ast.rs` live inline here.

#![allow(clippy::all)]

// ── Public surface: program ────────────────────────────────────────────────

/// A whole program: a list of top-level statements (variables, functions,
/// imports, exports).
#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

// ── Public surface: function & module support structs ──────────────────────

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Vec<Stmt>,
    /// Line (0-indexed) of the `function` keyword in the source.
    pub line: usize,
    /// Column (0-indexed) of the function name in the source.
    pub col: usize,
}

/// One named binding inside an `import { ... }`. `name` is the exported name;
/// `alias` is the local name (after `as`). If there is no `as`, the local
/// name equals `name`.
#[derive(Debug, Clone)]
pub struct ImportNamed {
    pub name: String,
    pub alias: Option<String>,
}

/// One item inside an `export { ... }` re-export list.
#[derive(Debug, Clone)]
pub struct ExportItem {
    pub name: String,
    pub alias: Option<String>,
}

/// `export default` can be a function (named or anonymous) or an expression.
#[derive(Debug, Clone)]
pub enum ExportDefault {
    /// If the source has `export default function f(){}`, `name` is `f`;
    /// for `export default function(){}` the name is empty and the codegen
    /// falls back to `__default`.
    Function(Function),
    Expr(Expr),
}

// ── Public surface: statements ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        ty: Option<Type>,
        value: Expr,
        line: usize,
        col: usize,
    },
    Const {
        name: String,
        ty: Option<Type>,
        value: Expr,
        line: usize,
        col: usize,
    },
    Assign {
        name: String,
        value: Expr,
    },
    AssignIndex {
        object: String,
        index: Expr,
        value: Expr,
    },
    /// Field assignment: `obj.field = value` or `arr[i].field = v`.
    AssignMember {
        object: Box<Expr>,
        property: String,
        value: Expr,
    },
    Function(Function),
    Return(Option<Expr>),
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    For {
        init: Option<Box<Stmt>>,
        condition: Option<Expr>,
        update: Option<Box<Stmt>>,
        body: Vec<Stmt>,
    },
    ForOf {
        name: String,
        ty: Option<Type>,
        iterable: Box<Expr>,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Expr(Expr),
    /// `switch (discriminant) { case v1: ... case v2: ... default: ... }`.
    /// Non-fallthrough: each case is its own block (like a Rust `match`
    /// arm), not a C-style fallthrough chain. `break;` inside a case is
    /// parsed and validated but is a codegen no-op — every case already
    /// ends its own arm.
    Switch {
        discriminant: Expr,
        cases: Vec<SwitchCase>,
    },
    /// `try { body } catch (e) { catch_body }`. Lowers to
    /// `std::panic::catch_unwind` — see `docs/language-reference.md` for the
    /// documented `AssertUnwindSafe` caveat.
    Try {
        body: Vec<Stmt>,
        catch_name: Option<String>,
        catch_body: Vec<Stmt>,
    },
    /// `throw expr;` — lowers to `panic!("{}", expr)`.
    Throw(Expr),

    // Modules (Python-style).
    /// `import utils` or `import utils as u`
    Import {
        module: Vec<String>,
        alias: Option<String>,
    },
    /// `from utils import a, b as c` or `from utils import *`
    FromImport {
        module: Vec<String>,
        names: Vec<ImportNamed>,
        wildcard: bool,
    },
    /// `export function f(){}` / `export const X = ...;` / `export let Y = ...;`
    ExportDecl(Box<Stmt>),
    /// `export { a, b as c };` — re-export of already-declared names.
    ExportSpec(Vec<ExportItem>),
    /// `export default ...` — function or expression.
    ExportDefault(ExportDefault),

    /// `type Name = <type>;`
    TypeAlias {
        name: String,
        ty: Type,
        line: usize,
        col: usize,
    },
    /// `interface Name [extends Base, ...] { field: type, field2?: type }`
    Interface {
        name: String,
        extends: Vec<String>,
        fields: Vec<(String, Box<Type>, bool)>,
        line: usize,
        col: usize,
    },
    /// `enum Name { A, B = 5, C }`. `variants` is `(variant_name, explicit_value)`
    /// in source order; an unset value means "previous value + 1" (or `0`
    /// for the first variant), matching TypeScript numeric enum rules.
    Enum {
        name: String,
        variants: Vec<(String, Option<i64>)>,
        line: usize,
        col: usize,
    },
}

/// One `case`/`default` arm of a [`Stmt::Switch`]. `values` holds every
/// value for a `case v1: case v2: ...` fallthrough-to-body group (matching
/// Rust's `v1 | v2 => { }` or-pattern); empty + `is_default` for `default:`.
#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub values: Vec<Expr>,
    pub body: Vec<Stmt>,
    pub is_default: bool,
}

// ── Public surface: expressions & operators ────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    String(String),
    Bool(bool),
    Ident(String),
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Member access: `s.length`. Only `.length` is translated in v1.
    Member {
        object: Box<Expr>,
        property: String,
    },
    /// Indexing: `arr[i]`.
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    /// Array literal: `[1, 2, 3]`, optionally with `...spread` elements.
    ArrayLiteral {
        elements: Vec<ArrayElement>,
    },
    /// Object literal: `{ key: value, ... }`, optionally with
    /// `...spread` entries. The type is inferred from the context
    /// (`let`/`const` with declared type) — the codegen resolves it.
    ObjectLiteral {
        fields: Vec<ObjectField>,
    },
    /// Static path: `reqwest::Client::new`. Translated to
    /// `seg1::seg2::...::segN`. If followed by `(args)` the parser wraps it
    /// into `Expr::Call { callee: Path, args }`.
    Path {
        segments: Vec<String>,
    },
    /// `typeof expr` — returns the compile-time type as a string.
    TypeOf(Box<Expr>),
    /// `null`
    Null,
    /// `undefined`
    Undefined,
    /// `expr as Type` — type assertion (no runtime effect).
    AsAssertion {
        expr: Box<Expr>,
        ty: Type,
    },
    /// `expr as const` — const assertion (literal-preserving), tracked
    /// separately from `AsAssertion` since it has no explicit target type.
    AsConst(Box<Expr>),
    /// `expr!` — non-null assertion (postfix `!`).
    NonNullAssertion(Box<Expr>),
    /// `(a: T, b: U): R => expr` or `(a: T) => { stmt* }`. No variable
    /// capture (see `docs/language-reference.md`) — codegen lowers this to
    /// a non-capturing Rust closure, which coerces to a plain `fn` pointer
    /// wherever one is expected. `return_type` is `None` when the source
    /// omits it — codegen then leaves it out of the Rust closure signature
    /// too, letting `rustc` infer it, rather than defaulting to `void`
    /// (which would be wrong for `(x: number) => x * 2`).
    Arrow {
        params: Vec<Param>,
        return_type: Option<Type>,
        body: ArrowBody,
    },
}

/// The body of an [`Expr::Arrow`]: either the short expression form
/// (`x => x * 2`) or a block form (`x => { return x * 2; }`).
#[derive(Debug, Clone)]
pub enum ArrowBody {
    Expr(Box<Expr>),
    Block(Vec<Stmt>),
}

/// One element of an [`Expr::ArrayLiteral`]: a plain item, or a
/// `...spread` of another array's elements.
#[derive(Debug, Clone)]
pub enum ArrayElement {
    Item(Expr),
    Spread(Expr),
}

/// One entry of an [`Expr::ObjectLiteral`]: a `key: value` pair, or a
/// `...spread` of another object's fields.
#[derive(Debug, Clone)]
pub enum ObjectField {
    KV(String, Expr),
    Spread(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

// ── Public surface: types ──────────────────────────────────────────────────

/// A literal value used in a literal type position (`"hello"`, `42`, `true`).
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    String(String),
    Number(f64),
    Bool(bool),
}

/// A language type as it appears in source.
///
/// This is a recursive enum so it can represent everything from TypeScript's
/// "Everyday Types": primitives, arrays, inline object types (optionally
/// with optional fields), unions, intersections, literal types, named
/// (user-defined) types, and function types.
///
/// Compatibility accessors (`is_array`, `array_inner`, `object_fields`,
/// `primitive_name`, `struct_name`) are provided so callers written against
/// the old flat struct can be ported incrementally.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// `string`, `number`, `boolean`, `void`, `any`, `bigint`, `symbol`, or
    /// any bare identifier used in a type position that isn't otherwise
    /// recognised (kept as `Named` normally — this variant is only for the
    /// built-in primitive keywords).
    Primitive(String),
    /// `null`
    Null,
    /// `undefined`
    Undefined,
    /// `T[]`
    Array(Box<Type>),
    /// `{ key: T, key2?: U }`. The `bool` flags an optional (`?`) field.
    Object {
        /// Deterministic, hash-based name (`__ObjHASH`) used by codegen to
        /// emit/reference a single `struct` per distinct shape.
        name: String,
        fields: Vec<(String, Box<Type>, bool)>,
    },
    /// `A | B | C`
    Union(Vec<Type>),
    /// `A & B`
    Intersection(Vec<Type>),
    /// `"hello"`, `42`, `true`
    Literal(LiteralValue),
    /// A user-defined / named type (interface name, type alias name, or any
    /// identifier not recognised as a primitive keyword).
    Named(String),
    /// `(a: T, b: U) => R`
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },
}

impl Type {
    pub fn string() -> Self {
        Type::Primitive("string".into())
    }
    pub fn number() -> Self {
        Type::Primitive("number".into())
    }
    pub fn boolean() -> Self {
        Type::Primitive("boolean".into())
    }
    pub fn void() -> Self {
        Type::Primitive("void".into())
    }
    pub fn any() -> Self {
        Type::Primitive("any".into())
    }
    pub fn array(inner: Type) -> Self {
        Type::Array(Box::new(inner))
    }

    /// `true` for the `T[]` shape (top-level only; does not recurse).
    pub fn is_array(&self) -> bool {
        matches!(self, Type::Array(_))
    }

    /// The element type of `T[]`, if this is an array type.
    pub fn array_inner(&self) -> Option<&Type> {
        match self {
            Type::Array(inner) => Some(inner),
            _ => None,
        }
    }

    /// The `(field_name, field_type, optional)` list, if this is an inline
    /// object type.
    pub fn object_fields(&self) -> Option<&[(String, Box<Type>, bool)]> {
        match self {
            Type::Object { fields, .. } => Some(fields),
            _ => None,
        }
    }

    /// `true` if this is an inline object type.
    pub fn is_object(&self) -> bool {
        matches!(self, Type::Object { .. })
    }

    /// The surface name of a primitive, named, or object type. Arrays return
    /// the inner element's name (matching the old struct's flattened
    /// `name` + `is_array` pair); unions/literals/functions return a
    /// synthetic label since they have no single "name".
    pub fn primitive_name(&self) -> &str {
        match self {
            Type::Primitive(n) => n,
            Type::Named(n) => n,
            Type::Object { name, .. } => name,
            Type::Array(inner) => inner.primitive_name(),
            Type::Null => "null",
            Type::Undefined => "undefined",
            Type::Union(_) => "union",
            Type::Intersection(_) => "intersection",
            Type::Literal(_) => "literal",
            Type::Function { .. } => "function",
        }
    }

    /// The struct name for an inline object type, if any (used by codegen
    /// to emit/reference `pub struct __ObjHASH`).
    pub fn struct_name(&self) -> Option<&str> {
        match self {
            Type::Object { name, .. } => Some(name),
            Type::Array(inner) => inner.struct_name(),
            _ => None,
        }
    }
}