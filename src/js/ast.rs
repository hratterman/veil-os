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
    /// Comma/sequence operator: evaluate each left-to-right, yield the last.
    Seq(Vec<Expr>),
    /// Regex literal /pattern/flags.
    Regex(String, String),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String, bool), // obj.prop, optional?
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>, bool), // callee, args, optional?.()
    New(Box<Expr>, Vec<Expr>),
    Func(Box<Func>),
    Arrow(Box<Func>),
    /// await expr (ES2017) — unwraps a resolved Promise.
    Await(Box<Expr>),
    /// yield [expr] inside a generator.
    Yield(Option<Box<Expr>>, bool /* delegate (yield*) */),
    /// class expression / declaration.
    Class(Box<Class>),
    /// super(...) call or super.method — `prop` None means a super() ctor call.
    Super(Option<String>),
}

#[derive(Clone, Debug)]
pub struct Class {
    pub name: Option<String>,
    pub parent: Option<Box<Expr>>,
    pub ctor: Option<Func>,
    /// (name, function, is_static, kind) where kind is "method"|"get"|"set".
    pub methods: Vec<(String, Func, bool, &'static str)>,
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
    /// `...expr` object spread; the paired value Expr is the spread source.
    Spread,
    /// getter/setter shorthand — value Expr is Func.
    Getter(String),
    Setter(String),
}

#[derive(Clone, Debug)]
pub enum Pat {
    Ident(String),
    /// array destructuring with optional rest as the last element
    Array(Vec<Pat>, Option<String>),
    /// object destructuring: (sourceKey, bindingPattern) pairs + optional rest
    Object(Vec<(String, Pat)>, Option<String>),
    /// a pattern with a default value (default params, destructuring defaults)
    Default(Box<Pat>, Box<Expr>),
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
    /// async function — its return value is wrapped in a resolved Promise.
    pub is_async: bool,
    /// generator function (function*).
    pub is_generator: bool,
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
    /// do { ... } while (cond) — body runs at least once.
    DoWhile(Expr, Vec<Stmt>),
    /// switch (disc) { case e: stmts ... default: stmts }. Each arm is
    /// (Some(test) | None for default, body). `break` is kept in the body so the
    /// evaluator can implement C-style fall-through (an arm with no `break` runs
    /// into the next arm).
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    Block(Vec<Stmt>),
    /// `break` or `break label` (the label targets an enclosing labeled loop).
    Break(Option<String>),
    /// `continue` or `continue label`.
    Continue(Option<String>),
    /// `label: stmt` — names the (usually loop) statement so a labeled
    /// break/continue can target it. Minifiers emit these to flatten control
    /// flow, so getting them right matters for production React.
    Labeled(String, Box<Stmt>),
    Throw(Expr),
    Try(Vec<Stmt>, Option<(Option<String>, Vec<Stmt>)>, Vec<Stmt>),
    Empty,
}
