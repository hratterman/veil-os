//! JavaScript runtime values.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::ast::Func;

pub type Obj = BTreeMap<String, Val>;

#[derive(Clone)]
pub enum Val {
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<String>),
    Array(Rc<RefCell<Vec<Val>>>),
    Object(Rc<RefCell<Obj>>),
    /// A user function with its captured scope.
    Func(Rc<Func>, super::interp::Scope),
    /// A native/bound function: (kind, optional receiver node).
    Native(Native),
    /// A DOM element handle (index into the arena), or a special host object.
    Node(usize),
    /// Host pseudo-objects (document, window, console, Math, localStorage, a
    /// node's .style / .classList / .dataset). Carries an owner node where
    /// relevant.
    Host(Host),
}

#[derive(Clone)]
pub enum Native {
    /// A bound method on a value, e.g. arr.map / str.split / node.appendChild.
    Method(Box<Val>, Rc<str>),
    /// A free global function, e.g. setTimeout / parseInt.
    Global(Rc<str>),
}

#[derive(Clone)]
pub enum Host {
    Document,
    Window,
    Console,
    Math,
    LocalStorage,
    SessionStorage,
    History,
    Location,
    /// element.style
    Style(usize),
    /// element.classList
    ClassList(usize),
    /// element.dataset
    Dataset(usize),
}

impl Val {
    pub fn str(s: impl Into<String>) -> Val {
        Val::Str(Rc::new(s.into()))
    }
    pub fn array(v: Vec<Val>) -> Val {
        Val::Array(Rc::new(RefCell::new(v)))
    }
    pub fn object(o: Obj) -> Val {
        Val::Object(Rc::new(RefCell::new(o)))
    }

    pub fn truthy(&self) -> bool {
        match self {
            Val::Undef | Val::Null => false,
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Str(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn as_num(&self) -> f64 {
        match self {
            Val::Num(n) => *n,
            Val::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Val::Str(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
            Val::Null => 0.0,
            _ => f64::NAN,
        }
    }

    pub fn to_str(&self) -> String {
        match self {
            Val::Undef => String::from("undefined"),
            Val::Null => String::from("null"),
            Val::Bool(b) => String::from(if *b { "true" } else { "false" }),
            Val::Num(n) => num_to_str(*n),
            Val::Str(s) => (**s).clone(),
            Val::Array(a) => {
                let items: Vec<String> = a.borrow().iter().map(|v| v.to_str()).collect();
                items.join(",")
            }
            Val::Object(_) => String::from("[object Object]"),
            _ => String::from("[object Object]"),
        }
    }
}

pub fn num_to_str(n: f64) -> String {
    if n.is_nan() {
        return String::from("NaN");
    }
    if n == 0.0 {
        return String::from("0");
    }
    if n == super::mathf::trunc(n) && n.abs() < 1e15 {
        return alloc::format!("{}", n as i64);
    }
    let mut s = alloc::format!("{}", n);
    // trim noise
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}
