//! Parser recursivo descendente con precedence climbing para expresiones.
//!
//! Gramática soportada (simplificada):
//!   program     = stmt*
//!   stmt        = let_stmt | const_stmt | fn_stmt | if_stmt | while_stmt
//!               | for_stmt | return_stmt | break_stmt | continue_stmt
//!               | assign_stmt | expr_stmt
//!   let_stmt    = "let"  IDENT (":" type)? "=" expr ";"
//!   const_stmt  = "const" IDENT (":" type)? "=" expr ";"
//!   fn_stmt     = "function" IDENT "(" params? ")" (":" type)? "{" stmt* "}"
//!   if_stmt     = "if" "(" expr ")" "{" stmt* "}" ("else" "{" stmt* "}")?
//!   while_stmt  = "while" "(" expr ")" "{" stmt* "}"
//!   for_stmt    = "for" "(" (let_stmt | expr_stmt | ";") expr? ";" expr? ")" "{" stmt* "}"
//!   return_stmt = "return" expr? ";"
//!   break_stmt  = "break" ";"
//!   continue_stmt = "continue" ";"
//!   expr_stmt   = expr ";"
//!
//!   expr        = or
//!   or          = and ( "||" and )*
//!   and         = equality ( "&&" equality )*
//!   equality    = comparison ( ("==" | "!=") comparison )*
//!   comparison  = additive ( ("<" | ">" | "<=" | ">=") additive )*
//!   additive    = multiplicative ( ("+" | "-") multiplicative )*
//!   multiplicative = unary ( ("*" | "/" | "%") unary )*
//!   unary       = ("!" | "-") unary | postfix
//!   postfix     = atom ( "." IDENT | "[" expr "]" )*
//!   atom        = NUMBER | STRING | "true" | "false" | IDENT ("(" args ")")? | "(" expr ")"

use crate::ast::{
    BinOp, ExportDefault, ExportItem, Expr, Function, ImportNamed, Param, Program, Stmt, Type,
    UnaryOp,
};
use crate::token::{Token, TokenKind};

/// Genera un identificador determinista para un tipo objeto inline,
/// basado en el hash del shape. Mismo shape → mismo nombre → mismo struct.
fn object_type_name(fields: &[(String, Box<Type>)]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (k, t) in fields {
        k.hash(&mut h);
        t.name.hash(&mut h);
        for (fk, ft) in &t.fields {
            fk.hash(&mut h);
            ft.name.hash(&mut h);
        }
    }
    format!("__Obj{:x}", h.finish() & 0xFFFFFF)
}

pub fn parse(tokens: Vec<Token>) -> Result<Program, ParseError> {
    let mut p = Parser { tokens, pos: 0 };
    let mut stmts = Vec::new();
    while !p.check(&TokenKind::Eof) {
        stmts.push(p.parse_stmt()?);
    }
    Ok(Program { stmts })
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

#[derive(Debug)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error de parseo en {}:{}: {}", self.line, self.col, self.msg)
    }
}

impl std::error::Error for ParseError {}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    /// Lookahead: devuelve el token en la posición `pos + offset`, sin consumir.
    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind)
    }

    fn matches(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, context: &str) -> Result<Token, ParseError> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            let t = self.peek();
            Err(ParseError {
                line: t.line,
                col: t.col,
                msg: format!("se esperaba {}, encontré {}", kind, context),
            })
        }
    }

    // ---------- Sentencias ----------

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek_kind() {
            TokenKind::Let => self.parse_let(false),
            TokenKind::Const => self.parse_let(true),
            TokenKind::Function => self.parse_function(),
            TokenKind::Import => self.parse_import(),
            TokenKind::Export => self.parse_export(),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Break => {
                self.advance();
                self.expect(&TokenKind::Semi, "`;` después de break")?;
                Ok(Stmt::Break)
            }
            TokenKind::Continue => {
                self.advance();
                self.expect(&TokenKind::Semi, "`;` después de continue")?;
                Ok(Stmt::Continue)
            }
            TokenKind::Ident(_) => {
                // Detectar asignación indexada: arr[expr] = expr;
                // O asignación de propiedad: obj.field = expr;
                // Si el token actual es Ident y el siguiente es `[` o `.`,
                // parseamos la expresión completa y luego vemos si hay `=`.
                let next_is_index_or_member = matches!(
                    self.peek_at(1).map(|t| &t.kind),
                    Some(TokenKind::LBracket) | Some(TokenKind::Dot)
                );
                if next_is_index_or_member {
                    let expr = self.parse_expr()?;
                    if self.check(&TokenKind::Eq) {
                        // Transformar Index/Member{lhs=Ident, ...} en Assign.
                        match expr {
                            Expr::Index { object, index } => {
                                let name = if let Expr::Ident(n) = *object {
                                    n
                                } else {
                                    return Err(ParseError {
                                        line: self.peek().line,
                                        col: self.peek().col,
                                        msg: "lado izquierdo de asignación indexada debe ser un identificador".to_string(),
                                    });
                                };
                                self.advance(); // =
                                let value = self.parse_expr()?;
                                self.expect(&TokenKind::Semi, "`;` después de la asignación indexada")?;
                                return Ok(Stmt::AssignIndex {
                                    object: name,
                                    index: *index,
                                    value,
                                });
                            }
                            Expr::Member { object, property } => {
                                self.advance(); // =
                                let value = self.parse_expr()?;
                                self.expect(&TokenKind::Semi, "`;` después de la asignación de campo")?;
                                return Ok(Stmt::AssignMember {
                                    object,
                                    property,
                                    value,
                                });
                            }
                            _ => {
                                return Err(ParseError {
                                    line: self.peek().line,
                                    col: self.peek().col,
                                    msg: "lado izquierdo de asignación inválido".to_string(),
                                });
                            }
                        }
                    }
                    self.expect(&TokenKind::Semi, "después de la expresión")?;
                    return Ok(Stmt::Expr(expr));
                }
                // Disambiguar: si después del ident viene `=` (no `==`),
                // es asignación. Si no, es una expresión.
                if matches!(
                    self.peek_at(1).map(|t| &t.kind),
                    Some(TokenKind::Eq)
                ) {
                    self.parse_assign()
                } else {
                    let expr = self.parse_expr()?;
                    self.expect(&TokenKind::Semi, "después de la expresión")?;
                    Ok(Stmt::Expr(expr))
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::Semi, "después de la expresión")?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn parse_assign(&mut self) -> Result<Stmt, ParseError> {
        let name_tok = self.advance(); // Ident
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!("parse_assign llamado sin Ident"),
        };
        self.expect(&TokenKind::Eq, "`=` después del nombre")?;
        let value = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "`;` después del valor")?;
        Ok(Stmt::Assign { name, value })
    }

    fn parse_let(&mut self, is_const: bool) -> Result<Stmt, ParseError> {
        self.advance(); // let / const
        let name_tok = self.expect(&TokenKind::Ident(String::new()), "nombre de la variable")?;
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        let line = name_tok.line;
        let col = name_tok.col;

        let ty = if self.matches(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&TokenKind::Eq, "asignación `=`")?;
        let value = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "después del valor")?;

        if is_const {
            Ok(Stmt::Const { name, ty, value, line, col })
        } else {
            Ok(Stmt::Let { name, ty, value, line, col })
        }
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        // Tipo objeto inline: { nombre: tipo, ... }
        if self.check(&TokenKind::LBrace) {
            self.advance(); // {
            let mut fields = Vec::new();
            // Permitir {} (objeto vacío)
            if !self.check(&TokenKind::RBrace) {
                loop {
                    let key_tok = self.expect(&TokenKind::Ident(String::new()), "nombre de campo en tipo objeto")?;
                    let key = match &key_tok.kind {
                        TokenKind::Ident(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    self.expect(&TokenKind::Colon, "`:` después del nombre de campo")?;
                    let field_ty = self.parse_type()?;
                    fields.push((key, Box::new(field_ty)));
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RBrace, "`}` cerrando tipo objeto")?;
            // Generar nombre único determinista basado en el shape.
            let name = object_type_name(&fields);
            let mut is_array = false;
            // Sufijo opcional `[]` (array de objetos).
            if self.check(&TokenKind::LBracket) {
                self.advance();
                self.expect(&TokenKind::RBracket, "`]` después de `[`")?;
                is_array = true;
            }
            return Ok(Type { name, fields, is_array });
        }

        let t = self.advance();
        let name = match &t.kind {
            TokenKind::TypeString => "string".to_string(),
            TokenKind::TypeNumber => "number".to_string(),
            TokenKind::TypeBoolean => "boolean".to_string(),
            TokenKind::TypeVoid => "void".to_string(),
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(ParseError {
                    line: t.line,
                    col: t.col,
                    msg: format!("tipo desconocido {}", other),
                });
            }
        };
        let mut is_array = false;
        // Sufijo opcional `[]` para tipo array (un nivel en v1).
        if self.check(&TokenKind::LBracket) {
            self.advance();
            self.expect(&TokenKind::RBracket, "`]` después de `[`")?;
            is_array = true;
        }
        Ok(Type { name, fields: Vec::new(), is_array })
    }

    fn parse_function(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // function
        let f = self.parse_function_rest(true)?;
        Ok(Stmt::Function(f))
    }

    /// Parsea `[nombre] (params): retorno { body }`.
    /// Se llama justo después de consumir `function`.
    /// Si `require_name` es false (caso `export default function`), permite
    /// una función anónima (nombre vacío).
    fn parse_function_rest(&mut self, require_name: bool) -> Result<Function, ParseError> {
        let name = if let TokenKind::Ident(_) = self.peek_kind() {
            let tok = self.advance();
            match tok.kind {
                TokenKind::Ident(s) => s,
                _ => unreachable!(),
            }
        } else if require_name {
            let t = self.peek();
            return Err(ParseError {
                line: t.line,
                col: t.col,
                msg: "se esperaba el nombre de la función".to_string(),
            });
        } else {
            String::new()
        };

        self.expect(&TokenKind::LParen, "`(` después del nombre")?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let pname_tok = self.expect(&TokenKind::Ident(String::new()), "nombre de parámetro")?;
                let pname = match &pname_tok.kind {
                    TokenKind::Ident(s) => s.clone(),
                    _ => unreachable!(),
                };
                let pline = pname_tok.line;
                let pcol = pname_tok.col;
                self.expect(&TokenKind::Colon, "`:` después del nombre")?;
                let pty = self.parse_type()?;
                params.push(Param { name: pname, ty: pty, line: pline, col: pcol });
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "`)` después de parámetros")?;

        let return_type = if self.matches(&TokenKind::Colon) {
            self.parse_type()?
        } else {
            Type::void()
        };

        self.expect(&TokenKind::LBrace, "`{{` inicio del cuerpo")?;
        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}}` fin del cuerpo")?;

        Ok(Function {
            name,
            params,
            return_type,
            body,
        })
    }

    // ---------- Imports / Exports (ES modules / TS) ----------

    /// Consume un token string literal y devuelve su contenido.
    fn expect_string_literal(&mut self, context: &str) -> Result<String, ParseError> {
        let t = self.peek().clone();
        if let TokenKind::String(s) = &t.kind {
            let s = s.clone();
            self.advance();
            Ok(s)
        } else {
            Err(ParseError {
                line: t.line,
                col: t.col,
                msg: format!("se esperaba un string ({})", context),
            })
        }
    }

    /// `import [def,] { a, b as c } from "mod";`
    /// Al menos uno de `def` (default) o la lista nombrada debe estar presente.
    fn parse_import(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // import

        let mut default: Option<String> = None;
        let mut named: Vec<ImportNamed> = Vec::new();

        // Binding default: identificador seguido de `,` o `from`.
        if let TokenKind::Ident(_) = self.peek_kind() {
            let after = self.peek_at(1).map(|t| &t.kind);
            if matches!(after, Some(TokenKind::From) | Some(TokenKind::Comma)) {
                let tok = self.advance();
                if let TokenKind::Ident(s) = tok.kind {
                    default = Some(s);
                }
            }
        }

        // `,` separador opcional entre default y la lista nombrada.
        if default.is_some() {
            self.matches(&TokenKind::Comma);
        }

        // Lista nombrada: `{ a, b as c }`
        if self.matches(&TokenKind::LBrace) {
            if !self.check(&TokenKind::RBrace) {
                loop {
                    let name_tok = self.expect(&TokenKind::Ident(String::new()), "nombre en import")?;
                    let name = match name_tok.kind {
                        TokenKind::Ident(s) => s,
                        _ => unreachable!(),
                    };
                    let alias = if self.matches(&TokenKind::As) {
                        let a = self.expect(&TokenKind::Ident(String::new()), "nombre después de `as`")?;
                        match a.kind {
                            TokenKind::Ident(s) => Some(s),
                            _ => unreachable!(),
                        }
                    } else {
                        None
                    };
                    named.push(ImportNamed { name, alias });
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RBrace, "`}` cerrando la lista de imports")?;
        }

        if default.is_none() && named.is_empty() {
            let t = self.peek();
            return Err(ParseError {
                line: t.line,
                col: t.col,
                msg: "import debe traer al menos un binding (default o nombrado)".to_string(),
            });
        }

        self.expect(&TokenKind::From, "`from` después de los bindings del import")?;
        let module = self.expect_string_literal("ruta del módulo")?;
        self.expect(&TokenKind::Semi, "`;` después del import")?;

        Ok(Stmt::Import { default, named, module })
    }

    /// `export default ... | export { ... } | export function/const/let ...`
    fn parse_export(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // export

        // export default ...
        if self.matches(&TokenKind::Default) {
            if self.check(&TokenKind::Function) {
                self.advance(); // function
                let f = self.parse_function_rest(false)?;
                return Ok(Stmt::ExportDefault(ExportDefault::Function(f)));
            }
            let expr = self.parse_expr()?;
            self.expect(&TokenKind::Semi, "`;` después de `export default`")?;
            return Ok(Stmt::ExportDefault(ExportDefault::Expr(expr)));
        }

        // export { a, b as c };
        if self.matches(&TokenKind::LBrace) {
            let mut items = Vec::new();
            if !self.check(&TokenKind::RBrace) {
                loop {
                    let name_tok = self.expect(&TokenKind::Ident(String::new()), "nombre en export")?;
                    let name = match name_tok.kind {
                        TokenKind::Ident(s) => s,
                        _ => unreachable!(),
                    };
                    let alias = if self.matches(&TokenKind::As) {
                        let a = self.expect(&TokenKind::Ident(String::new()), "nombre después de `as`")?;
                        match a.kind {
                            TokenKind::Ident(s) => Some(s),
                            _ => unreachable!(),
                        }
                    } else {
                        None
                    };
                    items.push(ExportItem { name, alias });
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RBrace, "`}` cerrando la lista de export")?;
            self.expect(&TokenKind::Semi, "`;` después del export")?;
            return Ok(Stmt::ExportSpec(items));
        }

        // export function / const / let (declaración inline)
        match self.peek_kind() {
            TokenKind::Function => {
                let stmt = self.parse_function()?;
                Ok(Stmt::ExportDecl(Box::new(stmt)))
            }
            TokenKind::Const => {
                let stmt = self.parse_let(true)?;
                Ok(Stmt::ExportDecl(Box::new(stmt)))
            }
            TokenKind::Let => {
                let stmt = self.parse_let(false)?;
                Ok(Stmt::ExportDecl(Box::new(stmt)))
            }
            other => {
                let t = self.peek();
                Err(ParseError {
                    line: t.line,
                    col: t.col,
                    msg: format!(
                        "export inválido: se esperaba `default`, `{{`, `function`, `const` o `let`, encontré {}",
                        other
                    ),
                })
            }
        }
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // return
        if self.check(&TokenKind::Semi) {
            self.advance();
            return Ok(Stmt::Return(None));
        }
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semi, "`;` después de return")?;
        Ok(Stmt::Return(Some(expr)))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // if
        self.expect(&TokenKind::LParen, "`(` después de if")?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "`)` después de la condición")?;
        self.expect(&TokenKind::LBrace, "`{{` inicio del bloque then")?;
        let mut then_branch = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            then_branch.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}}` fin del bloque then")?;

        let else_branch = if self.matches(&TokenKind::Else) {
            self.expect(&TokenKind::LBrace, "`{{` inicio del bloque else")?;
            let mut stmts = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
                stmts.push(self.parse_stmt()?);
            }
            self.expect(&TokenKind::RBrace, "`}}` fin del bloque else")?;
            Some(stmts)
        } else {
            None
        };

        Ok(Stmt::If { condition, then_branch, else_branch })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // while
        self.expect(&TokenKind::LParen, "`(` después de while")?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "`)` después de la condición")?;
        self.expect(&TokenKind::LBrace, "`{{` inicio del cuerpo del while")?;
        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}}` fin del cuerpo del while")?;
        Ok(Stmt::While { condition, body })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // for
        self.expect(&TokenKind::LParen, "`(` después de for")?;

        // Detectar for-of: `for (let IDENT of EXPR)`.
        // El patrón `let IDENT = EXPR` (C-style init) tiene `=` en peek_at(2),
        // mientras que for-of tiene `of` en peek_at(2).
        let is_for_of = matches!(self.peek_kind(), TokenKind::Let)
            && matches!(self.peek_at(1).map(|t| std::mem::discriminant(&t.kind)),
                       Some(d) if d == std::mem::discriminant(&TokenKind::Ident(String::new())))
            && matches!(self.peek_at(2).map(|t| &t.kind), Some(TokenKind::Of));

        if is_for_of {
            self.advance(); // let
            let name_tok = self.expect(&TokenKind::Ident(String::new()), "nombre de variable en for-of")?;
            let name = match &name_tok.kind {
                TokenKind::Ident(s) => s.clone(),
                _ => unreachable!(),
            };
            // Tipo opcional: `let x: number of arr`
            let ty = if self.matches(&TokenKind::Colon) {
                Some(self.parse_type()?)
            } else {
                None
            };
            self.expect(&TokenKind::Of, "`of` después del nombre en for-of")?;
            let iterable = self.parse_expr()?;
            self.expect(&TokenKind::RParen, "`)` después del iterable de for-of")?;
            self.expect(&TokenKind::LBrace, "`{{` inicio del cuerpo del for-of")?;
            let mut body = Vec::new();
            while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
                body.push(self.parse_stmt()?);
            }
            self.expect(&TokenKind::RBrace, "`}}` fin del cuerpo del for-of")?;
            return Ok(Stmt::ForOf { name, ty, iterable: Box::new(iterable), body });
        }

        // init: let | const | expr | vacío (cada uno terminado con ;)
        let init = if self.check(&TokenKind::Semi) {
            self.advance();
            None
        } else if matches!(self.peek_kind(), TokenKind::Let | TokenKind::Const) {
            let stmt = self.parse_let(matches!(self.peek_kind(), TokenKind::Const))?;
            // parse_let ya consumió el `;`
            Some(Box::new(stmt))
        } else {
            // expresión
            let expr = self.parse_expr()?;
            self.expect(&TokenKind::Semi, "`;` después de la init de for")?;
            Some(Box::new(Stmt::Expr(expr)))
        };

        // condition: opcional
        let condition = if self.check(&TokenKind::Semi) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(&TokenKind::Semi, "`;` después de la condición de for")?;

        // update: opcional
        let update = if self.check(&TokenKind::RParen) {
            None
        } else {
            // puede ser una asignación o una expr stmt
            // Detectamos asignación igual que en parse_stmt pero sin `;`
            let stmt = if let TokenKind::Ident(_) = self.peek_kind() {
                if matches!(self.peek_at(1).map(|t| &t.kind), Some(TokenKind::Eq)) {
                    self.parse_assign_no_semi()
                } else {
                    let expr = self.parse_expr()?;
                    Stmt::Expr(expr)
                }
            } else {
                let expr = self.parse_expr()?;
                Stmt::Expr(expr)
            };
            Some(Box::new(stmt))
        };

        self.expect(&TokenKind::RParen, "`)` después del update de for")?;
        self.expect(&TokenKind::LBrace, "`{{` inicio del cuerpo del for")?;
        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::Eof) {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, "`}}` fin del cuerpo del for")?;

        Ok(Stmt::For { init, condition, update, body })
    }

    /// Variante de parse_assign sin consumir el `;` final, usada por for(update).
    fn parse_assign_no_semi(&mut self) -> Stmt {
        let name_tok = self.advance();
        let name = match &name_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            _ => unreachable!(),
        };
        // Detectar asignación indexada: nombre[expr] = expr
        if self.check(&TokenKind::LBracket) {
            self.advance(); // [
            let index = self.parse_expr().unwrap();
            self.expect(&TokenKind::RBracket, "`]` en asignación indexada").unwrap();
            self.expect(&TokenKind::Eq, "`=` en asignación indexada").unwrap();
            let value = self.parse_expr().unwrap();
            return Stmt::AssignIndex { object: name, index, value };
        }
        self.expect(&TokenKind::Eq, "`=` después del nombre").unwrap();
        let value = self.parse_expr().unwrap();
        Stmt::Assign { name, value }
    }

    // ---------- Expresiones (precedence climbing) ----------

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Binary { op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while self.check(&TokenKind::And) {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::Binary { op: BinOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::EqEq => BinOp::EqEq,
                TokenKind::NotEq => BinOp::NotEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::LtEq => BinOp::LtEq,
                TokenKind::GtEq => BinOp::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Bang => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Not, operand: Box::new(operand) })
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary { op: UnaryOp::Neg, operand: Box::new(operand) })
            }
            _ => self.parse_postfix(),
        }
    }

    /// Parsea un postfix: parte de un átomo y consume `.ident`, `[expr]` y `(args)`
/// encadenados para construir Member/Index/Call.
fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
    let mut expr = self.parse_atom()?;
    loop {
        match self.peek_kind() {
            TokenKind::Dot => {
                self.advance();
                let prop_tok = self.expect(
                    &TokenKind::Ident(String::new()),
                    "nombre de propiedad después de `.`",
                )?;
                let property = match &prop_tok.kind {
                    TokenKind::Ident(s) => s.clone(),
                    _ => unreachable!(),
                };
                expr = Expr::Member { object: Box::new(expr), property };
            }
            TokenKind::LBracket => {
                self.advance(); // [
                let index = self.parse_expr()?;
                self.expect(&TokenKind::RBracket, "`]` después del índice")?;
                expr = Expr::Index { object: Box::new(expr), index: Box::new(index) };
            }
            TokenKind::LParen => {
                // Llamada sobre cualquier expresión: obj.method(args), f(args), etc.
                self.advance(); // (
                let mut args = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    loop {
                        args.push(self.parse_expr()?);
                        if !self.matches(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "`)` después de argumentos")?;
                expr = Expr::Call { callee: Box::new(expr), args };
            }
            _ => break,
        }
    }
    Ok(expr)
}

    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        let t = self.advance();
        match t.kind {
            TokenKind::Number(n) => Ok(Expr::Number(n)),
            TokenKind::String(s) => Ok(Expr::String(s)),
            TokenKind::Bool(b) => Ok(Expr::Bool(b)),
            TokenKind::Ident(name) => {
                // Path estático: `crate::Type::method`. Solo válido al inicio
                // de una expresión (no encadenado sobre el resultado de otra).
                if self.check(&TokenKind::ColonColon) {
                    let mut segments = vec![name];
                    while self.matches(&TokenKind::ColonColon) {
                        let next = self.expect(
                            &TokenKind::Ident(String::new()),
                            "segmento de path después de `::`",
                        )?;
                        let segment = match next.kind {
                            TokenKind::Ident(s) => s,
                            _ => unreachable!(),
                        };
                        segments.push(segment);
                    }
                    if self.check(&TokenKind::LParen) {
                        self.advance();
                        let mut args = Vec::new();
                        if !self.check(&TokenKind::RParen) {
                            loop {
                                args.push(self.parse_expr()?);
                                if !self.matches(&TokenKind::Comma) {
                                    break;
                                }
                            }
                        }
                        self.expect(&TokenKind::RParen, "`)` después de argumentos")?;
                        return Ok(Expr::Call {
                            callee: Box::new(Expr::Path { segments }),
                            args,
                        });
                    }
                    return Ok(Expr::Path { segments });
                }
                if self.check(&TokenKind::LParen) {
                    self.advance(); // (
                    let mut args = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.matches(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)` después de argumentos")?;
                    Ok(Expr::Call { callee: Box::new(Expr::Ident(name)), args })
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            TokenKind::LParen => {
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "`)` después de la expresión agrupada")?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                // Literal de array: [expr, expr, ...] o []
                // El `[` ya fue consumido por `self.advance()` arriba.
                let mut elements = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    loop {
                        elements.push(self.parse_expr()?);
                        if !self.matches(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RBracket, "`]` cerrando el array literal")?;
                Ok(Expr::ArrayLiteral { elements })
            }
            TokenKind::LBrace => {
                // Literal de objeto: { clave: expr, ... }
                // Requiere estar en un contexto con tipo declarado (let/const).
                // El codegen infiere el tipo del contexto.
                let mut fields = Vec::new();
                if !self.check(&TokenKind::RBrace) {
                    loop {
                        let key_tok = self.expect(&TokenKind::Ident(String::new()), "nombre de campo en objeto literal")?;
                        let key = match &key_tok.kind {
                            TokenKind::Ident(s) => s.clone(),
                            _ => unreachable!(),
                        };
                        self.expect(&TokenKind::Colon, "`:` después del nombre de campo")?;
                        let value = self.parse_expr()?;
                        fields.push((key, value));
                        if !self.matches(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}` cerrando objeto literal")?;
                Ok(Expr::ObjectLiteral { fields })
            }
            other => Err(ParseError {
                line: t.line,
                col: t.col,
                msg: format!("se esperaba una expresión, encontré {}", other),
            }),
        }
    }
}