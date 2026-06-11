//! M32-B: a small Scheme/Lisp interpreter — reader, evaluator with a
//! trampolined eval loop (tail-call optimisation for `if`/`begin`/lambda
//! bodies), lexical environments, and a set of builtins. no_std + alloc.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

// REPL output sink: `display`/`newline` append here; the UI drains it.
static mut OUTPUT: Option<String> = None;

fn out_push(s: &str) {
    unsafe {
        let o = (*core::ptr::addr_of_mut!(OUTPUT)).get_or_insert_with(String::new);
        o.push_str(s);
    }
}

/// Take and clear any text produced by `display`/`newline` since last call.
pub fn take_output() -> String {
    unsafe { (*core::ptr::addr_of_mut!(OUTPUT)).take().unwrap_or_default() }
}

#[derive(Clone)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Sym(String),
    Str(String),
    Nil,
    Pair(Rc<Value>, Rc<Value>),
    Lambda { params: Vec<String>, body: Rc<Vec<Value>>, env: Env },
    Builtin(&'static str),
}

pub type Env = Rc<RefCell<Scope>>;

pub struct Scope {
    vars: BTreeMap<String, Value>,
    parent: Option<Env>,
}

fn new_env(parent: Option<Env>) -> Env {
    Rc::new(RefCell::new(Scope { vars: BTreeMap::new(), parent }))
}

fn env_get(env: &Env, name: &str) -> Option<Value> {
    let s = env.borrow();
    if let Some(v) = s.vars.get(name) {
        Some(v.clone())
    } else if let Some(p) = &s.parent {
        env_get(p, name)
    } else {
        None
    }
}

// --- reader -------------------------------------------------------------------

fn tokenize(src: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            ';' => {
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '\n' {
                        break;
                    }
                }
            }
            '(' | ')' | '\'' => {
                toks.push(c.to_string());
                chars.next();
            }
            '"' => {
                chars.next();
                let mut s = String::from("\"");
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                toks.push(s);
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' || c == '\'' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                toks.push(s);
            }
        }
    }
    toks
}

fn parse_all(src: &str) -> Result<Vec<Value>, String> {
    let toks = tokenize(src);
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < toks.len() {
        out.push(read(&toks, &mut pos)?);
    }
    Ok(out)
}

fn read(toks: &[String], pos: &mut usize) -> Result<Value, String> {
    let t = toks.get(*pos).ok_or("unexpected end of input")?.clone();
    *pos += 1;
    match t.as_str() {
        "(" => {
            let mut items = Vec::new();
            while toks.get(*pos).map(String::as_str) != Some(")") {
                if *pos >= toks.len() {
                    return Err("missing )".to_string());
                }
                items.push(read(toks, pos)?);
            }
            *pos += 1; // consume )
            Ok(list_from(items))
        }
        ")" => Err("unexpected )".to_string()),
        "'" => Ok(list_from(vec![Value::Sym("quote".into()), read(toks, pos)?])),
        _ => Ok(atom(&t)),
    }
}

fn atom(t: &str) -> Value {
    if let Some(s) = t.strip_prefix('"') {
        return Value::Str(s.to_string());
    }
    match t {
        "#t" => Value::Bool(true),
        "#f" => Value::Bool(false),
        _ => {
            if let Ok(n) = t.parse::<i64>() {
                Value::Int(n)
            } else {
                Value::Sym(t.to_string())
            }
        }
    }
}

fn list_from(items: Vec<Value>) -> Value {
    let mut v = Value::Nil;
    for item in items.into_iter().rev() {
        v = Value::Pair(Rc::new(item), Rc::new(v));
    }
    v
}

fn list_to_vec(mut v: &Value) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    loop {
        match v {
            Value::Nil => return Ok(out),
            Value::Pair(a, b) => {
                out.push((**a).clone());
                v = b;
            }
            _ => return Err("improper list".to_string()),
        }
    }
}

// --- printer ------------------------------------------------------------------

pub fn print_value(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => if *b { "#t".into() } else { "#f".into() },
        Value::Sym(s) => s.clone(),
        Value::Str(s) => format!("\"{s}\""),
        Value::Nil => "()".into(),
        Value::Lambda { .. } => "#<lambda>".into(),
        Value::Builtin(n) => format!("#<builtin:{n}>"),
        Value::Pair(..) => {
            let mut s = String::from("(");
            let mut cur = v.clone();
            let mut first = true;
            loop {
                match cur {
                    Value::Pair(a, b) => {
                        if !first {
                            s.push(' ');
                        }
                        first = false;
                        s.push_str(&print_value(&a));
                        cur = (*b).clone();
                    }
                    Value::Nil => break,
                    other => {
                        s.push_str(" . ");
                        s.push_str(&print_value(&other));
                        break;
                    }
                }
            }
            s.push(')');
            s
        }
    }
}

fn display_str(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(), // display strips the quotes
        _ => print_value(v),
    }
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Bool(false))
}

// --- evaluator ----------------------------------------------------------------

const BUILTINS: &[&str] = &[
    "+", "-", "*", "/", "mod", "=", "<", ">", "<=", ">=", "cons", "car", "cdr",
    "list", "null?", "pair?", "length", "append", "number?", "string?",
    "symbol?", "boolean?", "display", "newline", "not", "eq?", "equal?", "map",
    "help",
];

pub struct Interp {
    pub global: Env,
}

impl Interp {
    pub fn new() -> Interp {
        let global = new_env(None);
        {
            let mut g = global.borrow_mut();
            for &b in BUILTINS {
                g.vars.insert(b.to_string(), Value::Builtin(b));
            }
        }
        Interp { global }
    }

    /// Parse + evaluate a source string, returning the printed last result.
    pub fn eval_str(&mut self, src: &str) -> Result<String, String> {
        let exprs = parse_all(src)?;
        let mut last = Value::Nil;
        for e in exprs {
            last = eval(e, self.global.clone())?;
        }
        Ok(print_value(&last))
    }
}

fn eval(mut expr: Value, mut env: Env) -> Result<Value, String> {
    loop {
        match expr {
            Value::Int(_)
            | Value::Bool(_)
            | Value::Str(_)
            | Value::Nil
            | Value::Lambda { .. }
            | Value::Builtin(_) => return Ok(expr),
            Value::Sym(ref s) => {
                return env_get(&env, s).ok_or_else(|| format!("unbound: {s}"));
            }
            Value::Pair(ref head, ref tail) => {
                let head = (**head).clone();
                let args = list_to_vec(tail)?;
                if let Value::Sym(op) = &head {
                    match op.as_str() {
                        "quote" => return Ok(args.into_iter().next().unwrap_or(Value::Nil)),
                        "if" => {
                            let c = eval(args[0].clone(), env.clone())?;
                            expr = if truthy(&c) {
                                args[1].clone()
                            } else {
                                args.get(2).cloned().unwrap_or(Value::Nil)
                            };
                            continue;
                        }
                        "define" => return eval_define(&args, &env),
                        "lambda" => return make_lambda(&args, &env),
                        "begin" => {
                            if args.is_empty() {
                                return Ok(Value::Nil);
                            }
                            for e in &args[..args.len() - 1] {
                                eval(e.clone(), env.clone())?;
                            }
                            expr = args.last().unwrap().clone();
                            continue;
                        }
                        "let" => {
                            let scope = new_env(Some(env.clone()));
                            for binding in list_to_vec(&args[0])? {
                                let b = list_to_vec(&binding)?;
                                let name = sym_name(&b[0])?;
                                let v = eval(b[1].clone(), env.clone())?;
                                scope.borrow_mut().vars.insert(name, v);
                            }
                            env = scope;
                            expr = Value::Pair(
                                Rc::new(Value::Sym("begin".into())),
                                Rc::new(list_from(args[1..].to_vec())),
                            );
                            continue;
                        }
                        "cond" => {
                            let mut chosen = None;
                            for clause in &args {
                                let c = list_to_vec(clause)?;
                                let take = matches!(&c[0], Value::Sym(s) if s == "else")
                                    || truthy(&eval(c[0].clone(), env.clone())?);
                                if take {
                                    chosen = Some(list_from(
                                        core::iter::once(Value::Sym("begin".into()))
                                            .chain(c[1..].iter().cloned())
                                            .collect(),
                                    ));
                                    break;
                                }
                            }
                            expr = chosen.unwrap_or(Value::Nil);
                            continue;
                        }
                        "and" => {
                            let mut last = Value::Bool(true);
                            for e in &args {
                                last = eval(e.clone(), env.clone())?;
                                if !truthy(&last) {
                                    return Ok(Value::Bool(false));
                                }
                            }
                            return Ok(last);
                        }
                        "or" => {
                            for e in &args {
                                let v = eval(e.clone(), env.clone())?;
                                if truthy(&v) {
                                    return Ok(v);
                                }
                            }
                            return Ok(Value::Bool(false));
                        }
                        _ => {}
                    }
                }
                // Application.
                let f = eval(head, env.clone())?;
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(eval(a, env.clone())?);
                }
                match f {
                    Value::Builtin(name) => return apply_builtin(name, argv),
                    Value::Lambda { params, body, env: closure } => {
                        let scope = new_env(Some(closure));
                        bind_params(&scope, &params, argv)?;
                        if body.is_empty() {
                            return Ok(Value::Nil);
                        }
                        for e in &body[..body.len() - 1] {
                            eval(e.clone(), scope.clone())?;
                        }
                        expr = body.last().unwrap().clone();
                        env = scope;
                        continue;
                    }
                    other => return Err(format!("not callable: {}", print_value(&other))),
                }
            }
        }
    }
}

fn sym_name(v: &Value) -> Result<String, String> {
    match v {
        Value::Sym(s) => Ok(s.clone()),
        _ => Err("expected a symbol".to_string()),
    }
}

fn bind_params(scope: &Env, params: &[String], argv: Vec<Value>) -> Result<(), String> {
    if params.len() != argv.len() {
        return Err(format!("arity: want {} got {}", params.len(), argv.len()));
    }
    let mut s = scope.borrow_mut();
    for (p, a) in params.iter().zip(argv) {
        s.vars.insert(p.clone(), a);
    }
    Ok(())
}

fn make_lambda(args: &[Value], env: &Env) -> Result<Value, String> {
    let params = list_to_vec(&args[0])?
        .iter()
        .map(sym_name)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Lambda {
        params,
        body: Rc::new(args[1..].to_vec()),
        env: env.clone(),
    })
}

fn eval_define(args: &[Value], env: &Env) -> Result<Value, String> {
    match &args[0] {
        // (define name expr)
        Value::Sym(name) => {
            let v = eval(args[1].clone(), env.clone())?;
            env.borrow_mut().vars.insert(name.clone(), v);
            Ok(Value::Sym(name.clone()))
        }
        // (define (name params...) body...)  -> lambda shorthand
        Value::Pair(..) => {
            let sig = list_to_vec(&args[0])?;
            let name = sym_name(&sig[0])?;
            let lam = Value::Lambda {
                params: sig[1..].iter().map(sym_name).collect::<Result<_, _>>()?,
                body: Rc::new(args[1..].to_vec()),
                env: env.clone(),
            };
            env.borrow_mut().vars.insert(name.clone(), lam);
            Ok(Value::Sym(name))
        }
        _ => Err("bad define".to_string()),
    }
}

/// Apply a function value to already-evaluated arguments (non-tail; used by
/// higher-order builtins like `map`).
fn apply(f: Value, argv: Vec<Value>) -> Result<Value, String> {
    match f {
        Value::Builtin(name) => apply_builtin(name, argv),
        Value::Lambda { params, body, env } => {
            let scope = new_env(Some(env));
            bind_params(&scope, &params, argv)?;
            let mut last = Value::Nil;
            for e in body.iter() {
                last = eval(e.clone(), scope.clone())?;
            }
            Ok(last)
        }
        other => Err(format!("not callable: {}", print_value(&other))),
    }
}

fn as_int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(format!("expected a number, got {}", print_value(v))),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Sym(x), Value::Sym(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Nil, Value::Nil) => true,
        (Value::Pair(a1, a2), Value::Pair(b1, b2)) => {
            values_equal(a1, b1) && values_equal(a2, b2)
        }
        _ => false,
    }
}

const HELP: &str = "examples:\n  (+ 1 2 3)\n  (define (sq x) (* x x))  (sq 7)\n  (map (lambda (x) (* x x)) (list 1 2 3 4 5))\n  (if (< 3 5) 'yes 'no)\n  (define (fact n) (if (= n 0) 1 (* n (fact (- n 1)))))  (fact 10)";

fn apply_builtin(name: &str, a: Vec<Value>) -> Result<Value, String> {
    let ints = || a.iter().map(as_int).collect::<Result<Vec<_>, _>>();
    match name {
        "+" => Ok(Value::Int(ints()?.iter().sum())),
        "*" => Ok(Value::Int(ints()?.iter().product())),
        "-" => {
            let n = ints()?;
            Ok(Value::Int(match n.as_slice() {
                [] => 0,
                [x] => -x,
                [x, rest @ ..] => x - rest.iter().sum::<i64>(),
            }))
        }
        "/" => {
            let n = ints()?;
            let mut it = n.iter();
            let mut acc = *it.next().ok_or("/: need args")?;
            for &d in it {
                if d == 0 {
                    return Err("division by zero".to_string());
                }
                acc /= d;
            }
            Ok(Value::Int(acc))
        }
        "mod" => {
            let n = ints()?;
            if n[1] == 0 {
                return Err("mod by zero".to_string());
            }
            Ok(Value::Int(n[0].rem_euclid(n[1])))
        }
        "=" | "<" | ">" | "<=" | ">=" => {
            let n = ints()?;
            let ok = n.windows(2).all(|w| match name {
                "=" => w[0] == w[1],
                "<" => w[0] < w[1],
                ">" => w[0] > w[1],
                "<=" => w[0] <= w[1],
                _ => w[0] >= w[1],
            });
            Ok(Value::Bool(ok))
        }
        "cons" => Ok(Value::Pair(Rc::new(a[0].clone()), Rc::new(a[1].clone()))),
        "car" => match &a[0] {
            Value::Pair(x, _) => Ok((**x).clone()),
            _ => Err("car: not a pair".to_string()),
        },
        "cdr" => match &a[0] {
            Value::Pair(_, x) => Ok((**x).clone()),
            _ => Err("cdr: not a pair".to_string()),
        },
        "list" => Ok(list_from(a)),
        "null?" => Ok(Value::Bool(matches!(a[0], Value::Nil))),
        "pair?" => Ok(Value::Bool(matches!(a[0], Value::Pair(..)))),
        "length" => Ok(Value::Int(list_to_vec(&a[0])?.len() as i64)),
        "append" => {
            let mut items = Vec::new();
            for l in &a {
                items.extend(list_to_vec(l)?);
            }
            Ok(list_from(items))
        }
        "number?" => Ok(Value::Bool(matches!(a[0], Value::Int(_)))),
        "string?" => Ok(Value::Bool(matches!(a[0], Value::Str(_)))),
        "symbol?" => Ok(Value::Bool(matches!(a[0], Value::Sym(_)))),
        "boolean?" => Ok(Value::Bool(matches!(a[0], Value::Bool(_)))),
        "not" => Ok(Value::Bool(!truthy(&a[0]))),
        "eq?" | "equal?" => Ok(Value::Bool(values_equal(&a[0], &a[1]))),
        "display" => {
            out_push(&display_str(&a[0]));
            Ok(Value::Nil)
        }
        "newline" => {
            out_push("\n");
            Ok(Value::Nil)
        }
        "map" => {
            let f = a[0].clone();
            let items = list_to_vec(&a[1])?;
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(apply(f.clone(), vec![it])?);
            }
            Ok(list_from(out))
        }
        "help" => {
            out_push(HELP);
            out_push("\n");
            Ok(Value::Nil)
        }
        _ => Err(format!("unknown builtin: {name}")),
    }
}
