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

    // Modules (ES modules / TS).
    /// `import [def,] { a, b as c } from "mod";`
    Import {
        default: Option<String>,
        named: Vec<ImportNamed>,
        module: String,
    },
    /// `export function f(){}` / `export const X = ...;` / `export let Y = ...;`
    ExportDecl(Box<Stmt>),
    /// `export { a, b as c };` — re-export of already-declared names.
    ExportSpec(Vec<ExportItem>),
    /// `export default ...` — function or expression.
    ExportDefault(ExportDefault),
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
    /// Array literal: `[1, 2, 3]`.
    ArrayLiteral {
        elements: Vec<Expr>,
    },
    /// Object literal: `{ key: value, ... }`. The type is inferred from the
    /// context (`let`/`const` with declared type) — the codegen resolves it.
    ObjectLiteral {
        fields: Vec<(String, Expr)>,
    },
    /// Static path: `reqwest::Client::new`. Translated to
    /// `seg1::seg2::...::segN`. If followed by `(args)` the parser wraps it
    /// into `Expr::Call { callee: Path, args }`.
    Path {
        segments: Vec<String>,
    },
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

/// A language type as it appears in source.
///
/// For primitive types or arrays: only `name` (e.g. `"string"`, `"number"`).
/// For inline object types: `name` is a deterministic identifier (a hash of
/// the shape) and `fields` lists `(field_name, field_type)`.
/// `is_array` flags the `T[]` suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type {
    pub name: String,
    pub fields: Vec<(String, Box<Type>)>,
    pub is_array: bool,
}

impl Type {
    pub fn string() -> Self {
        Self {
            name: "string".into(),
            fields: Vec::new(),
            is_array: false,
        }
    }
    pub fn number() -> Self {
        Self {
            name: "number".into(),
            fields: Vec::new(),
            is_array: false,
        }
    }
    pub fn boolean() -> Self {
        Self {
            name: "boolean".into(),
            fields: Vec::new(),
            is_array: false,
        }
    }
    pub fn void() -> Self {
        Self {
            name: "void".into(),
            fields: Vec::new(),
            is_array: false,
        }
    }
}