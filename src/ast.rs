//! AST (Abstract Syntax Tree) del lenguaje.
//!
//! Representa la estructura semántica de un programa .tsr después del
//! parsing. El codegen traduce este árbol a código Rust.

/// Programa = lista de sentencias top-level (variables, funciones).
#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

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

/// Un binding nombrado dentro de un `import { ... }`.
/// `name` es el nombre exportado; `alias` es el nombre local (con `as`).
/// Si no hay `as`, el local es igual a `name`.
#[derive(Debug, Clone)]
pub struct ImportNamed {
    pub name: String,
    pub alias: Option<String>,
}

/// Un item dentro de un `export { ... }` (re-export de un nombre ya declarado).
/// `name` es el nombre local; `alias` es el nombre exportado (con `as`).
#[derive(Debug, Clone)]
pub struct ExportItem {
    pub name: String,
    pub alias: Option<String>,
}

/// `export default` puede ser una función (nombrada o anónima) o una expresión.
#[derive(Debug, Clone)]
pub enum ExportDefault {
    /// Si viene `export default function f(){}` el nombre es `f`; si es
    /// `export default function(){}` el nombre queda vacío y el codegen usa
    /// `__default`.
    Function(Function),
    Expr(Expr),
}

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
    /// Asignación a propiedad de objeto: `obj.field = value` o `arr[i].field = v`.
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

    // --- Módulos (ES modules / TS) ---
    /// `import [def,] { a, b as c } from "mod";`
    /// `default` es el binding local del default export (si lo hay).
    /// `named` son los bindings nombrados. Al menos uno de los dos está.
    Import {
        default: Option<String>,
        named: Vec<ImportNamed>,
        module: String,
    },
    /// `export function f(){}` / `export const X = ...;` / `export let Y = ...;`
    /// Envuelve la declaración ya parseada.
    ExportDecl(Box<Stmt>),
    /// `export { a, b as c };` — re-export de nombres ya declarados.
    ExportSpec(Vec<ExportItem>),
    /// `export default ...` — función o expresión.
    ExportDefault(ExportDefault),
}

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
    /// Acceso a miembro: `s.length`. Solo `.length` se traduce en v1.
    Member {
        object: Box<Expr>,
        property: String,
    },
    /// Indexing: `arr[i]`.
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    /// Literal de array: `[1, 2, 3]`.
    ArrayLiteral {
        elements: Vec<Expr>,
    },
    /// Literal de objeto: `{ clave: valor, ... }`. El tipo se infiere del
    /// contexto (let/const con tipo declarado) — el codegen lo resuelve.
    ObjectLiteral {
        fields: Vec<(String, Expr)>,
    },
    /// Path estático: `reqwest::Client::new`. Se traduce a `seg1::seg2::...::segN`.
    /// Si va seguido de `(args)` el parser lo envuelve en
    /// `Expr::Call { callee: Path, args }`.
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

/// Tipo del lenguaje tal como aparece en el código fuente TS-like.
///
/// Para tipos primitivos o arrays: solo `name` (e.g. "string", "number").
/// Para tipos objeto: `name` es un identificador único del struct (hash) y
/// `fields` lista `(nombre_campo, tipo_campo)`.
/// `is_array` indica que es `T[]` (envoltura de array).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type {
    pub name: String,
    pub fields: Vec<(String, Box<Type>)>,
    pub is_array: bool,
}

impl Type {
    pub fn string() -> Self { Self { name: "string".into(), fields: Vec::new(), is_array: false } }
    pub fn number() -> Self { Self { name: "number".into(), fields: Vec::new(), is_array: false } }
    pub fn boolean() -> Self { Self { name: "boolean".into(), fields: Vec::new(), is_array: false } }
    pub fn void() -> Self { Self { name: "void".into(), fields: Vec::new(), is_array: false } }
}