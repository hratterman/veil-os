//! JavaScript AST.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub enum Expr {
    Num(f64),
    Str(String),
    /// Template literal: alternating string chunks and interpolated exprs.
    Tmpl(Vec<TplElem>),
    Bool(bool),
    Null,
    Undef,
    Ident(String),
    This,
    Array(Vec<Expr>),
    Object(Vec<(PropKey, Expr)>),
    /// Spread element inside array/call, e.g. ...rest
    Spread(Box<Expr>),
    Unary(&'static str, Box<Expr>),
    /// prefix/postfix ++/--
    Update(&'static str, bool, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Logical(&'static str, Box<Expr>, Box<Expr>),
    Assign(&'static str, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String, bool), // obj.prop, optional?
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>, bool), // callee, args, optional?.()
    New(Box<Expr>, Vec<Expr>),
    Func(Box<Func>),
    Arrow(Box<Func>),
}

#[derive(Clone, Debug)]
pub enum TplElem {
    Str(String),
    Expr(Box<Expr>),
}

#[derive(Clone, Debug)]
pub enum PropKey {
    Ident(String),
    Computed(Box<Expr>),
}

#[derive(Clone, Debug)]
pub enum Pat {
    Ident(String),
    /// array destructuring with optional rest as the last element
    Array(Vec<Pat>, Option<String>),
}

#[derive(Clone, Debug)]
pub struct Func {
    pub name: Option<String>,
    pub params: Vec<Pat>,
    pub body: Vec<Stmt>,
    /// arrow with expression body
    pub expr_body: Option<Box<Expr>>,
    /// true for arrow functions (lexical `this`, no own `arguments`)
    pub arrow: bool,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr),
    /// (kind, [(pattern, init)])
    Decl(Vec<(Pat, Option<Expr>)>),
    FuncDecl(Box<Func>),
    Return(Option<Expr>),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    For(Box<Option<Stmt>>, Option<Expr>, Option<Expr>, Vec<Stmt>),
    /// for (x of iterable) { ... }
    ForOf(Pat, Expr, Vec<Stmt>),
    ForIn(Pat, Expr, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    Block(Vec<Stmt>),
    Break,
    Continue,
    Throw(Expr),
    Try(Vec<Stmt>, Option<(Option<String>, Vec<Stmt>)>, Vec<Stmt>),
    Empty,
}
