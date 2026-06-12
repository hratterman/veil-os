//! M41 real shell: a bash-compatible subset run as a tree-walking interpreter.
//!
//! Tokenizer -> recursive-descent parser (AST) -> executor. Supports: variable
//! expansion (`$VAR`, `${VAR}`, `${VAR:-def}`, `$?`, `$#`, `$@`, `$1..`), command
//! substitution `$(...)` / backticks, arithmetic `$((...))` + `let`, single/double
//! quoting + escapes, glob `*?[...]` against the FAT16 root, pipes, redirections
//! (`>`, `>>`, `<`, `2>`, `2>&1`), `&&`/`||`/`;`, `if/elif/else/fi`,
//! `for..in..do..done`, `while/until..do..done`, `case..esac`, functions
//! (`name() { }`), and builtins (`cd pwd echo printf export unset read source .
//! exit true false test [ [[ let : set shift type which`). The leaf file
//! commands (ls/cat/cp/mv/rm/grep/head/tail/sort/wc/find/date/df) operate on the
//! FAT16 disk. Background `&` and signals are cooperative (commands run
//! synchronously in the desktop task), documented in PROGRESS.

use crate::fs;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

const HELP: &str = "veil shell (bash subset):\n  \
files: ls cat cp mv rm find  |  text: grep head tail sort wc echo printf\n  \
vars: VAR=val  export VAR=val  $VAR ${VAR} $(cmd) $((expr))  unset\n  \
control: if/elif/else/fi  for x in ..; do ..; done  while/until  case..esac\n  \
funcs: name() { ..; }   tests: test/[ -f -d -z = -eq ..   redirect: > >> < 2>\n  \
builtins: cd pwd read source(.) let exit true false type which  run <app>\n";

/// What a command line produced: text to print and an optional app to launch.
pub struct Outcome {
    pub out: String,
    pub launch: Option<String>,
    pub clear: bool,
}

// --- persistent shell state ---------------------------------------------------

struct ShellState {
    vars: BTreeMap<String, String>,
    funcs: BTreeMap<String, Node>,
    status: i32,
    params: Vec<String>, // positional $1.. ($0 is "vsh")
    launch: Option<String>,
    clear: bool,
    exited: bool,
}

static mut STATE: Option<ShellState> = None;

fn state() -> &'static mut ShellState {
    unsafe {
        let s = &mut *core::ptr::addr_of_mut!(STATE);
        if s.is_none() {
            let mut vars = BTreeMap::new();
            vars.insert("USER".into(), "guest".into());
            vars.insert("SHELL".into(), "/bin/vsh".into());
            vars.insert("HOME".into(), "/".into());
            vars.insert("PWD".into(), "/".into());
            vars.insert("PATH".into(), "/bin".into());
            *s = Some(ShellState {
                vars,
                funcs: BTreeMap::new(),
                status: 0,
                params: Vec::new(),
                launch: None,
                clear: false,
                exited: false,
            });
        }
        s.as_mut().unwrap()
    }
}

/// Run a full command line / script fragment.
pub fn run(line: &str) -> Outcome {
    let st = state();
    st.launch = None;
    st.clear = false;
    st.exited = false;
    let toks = tokenize(line);
    let mut p = Parser { toks: &toks, pos: 0 };
    let mut out = String::new();
    while !p.at_end() {
        p.skip_seps();
        if p.at_end() {
            break;
        }
        match p.parse_and_or() {
            Some(node) => {
                let (o, status) = exec(&node, st, None);
                out.push_str(&o);
                st.status = status;
            }
            None => {
                // Unparseable token: skip it to avoid a hang.
                p.pos += 1;
            }
        }
        if st.exited {
            break;
        }
    }
    Outcome { out, launch: st.launch.take(), clear: st.clear }
}

// --- tokenizer ----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Op(&'static str), // | || && ; & ( ) < > >> 2> 2>&1 ;;
    Newline,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let b: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let n = b.len();
    while i < n {
        let c = b[i];
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            continue;
        }
        if c == '\n' || c == ';' {
            if c == ';' && i + 1 < n && b[i + 1] == ';' {
                toks.push(Tok::Op(";;"));
                i += 2;
            } else {
                toks.push(if c == '\n' { Tok::Newline } else { Tok::Op(";") });
                i += 1;
            }
            continue;
        }
        if c == '#' {
            // comment to end of line
            while i < n && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // operators
        if c == '|' {
            if i + 1 < n && b[i + 1] == '|' {
                toks.push(Tok::Op("||"));
                i += 2;
            } else {
                toks.push(Tok::Op("|"));
                i += 1;
            }
            continue;
        }
        if c == '&' {
            if i + 1 < n && b[i + 1] == '&' {
                toks.push(Tok::Op("&&"));
                i += 2;
            } else {
                toks.push(Tok::Op("&"));
                i += 1;
            }
            continue;
        }
        if c == '>' {
            if i + 1 < n && b[i + 1] == '>' {
                toks.push(Tok::Op(">>"));
                i += 2;
            } else {
                toks.push(Tok::Op(">"));
                i += 1;
            }
            continue;
        }
        if c == '<' {
            toks.push(Tok::Op("<"));
            i += 1;
            continue;
        }
        if c == '(' {
            toks.push(Tok::Op("("));
            i += 1;
            continue;
        }
        if c == ')' {
            toks.push(Tok::Op(")"));
            i += 1;
            continue;
        }
        // `2>` / `2>&1`
        if c == '2' && i + 1 < n && b[i + 1] == '>' {
            if i + 3 < n && b[i + 2] == '&' && b[i + 3] == '1' {
                toks.push(Tok::Op("2>&1"));
                i += 4;
            } else {
                toks.push(Tok::Op("2>"));
                i += 2;
            }
            continue;
        }
        // a word: accumulate until a separator/operator, honoring quotes and $(...)
        let mut w = String::new();
        while i < n {
            let c = b[i];
            if c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == ';' {
                break;
            }
            if matches!(c, '|' | '&' | '>' | '<') {
                break;
            }
            if c == '(' || c == ')' {
                // ')' / '(' end a word unless inside it's part of $(...) handled below
                break;
            }
            if c == '\'' {
                w.push(c);
                i += 1;
                while i < n && b[i] != '\'' {
                    w.push(b[i]);
                    i += 1;
                }
                if i < n {
                    w.push('\'');
                    i += 1;
                }
                continue;
            }
            if c == '"' {
                w.push(c);
                i += 1;
                while i < n && b[i] != '"' {
                    if b[i] == '\\' && i + 1 < n {
                        w.push(b[i]);
                        w.push(b[i + 1]);
                        i += 2;
                        continue;
                    }
                    w.push(b[i]);
                    i += 1;
                }
                if i < n {
                    w.push('"');
                    i += 1;
                }
                continue;
            }
            if c == '`' {
                w.push(c);
                i += 1;
                while i < n && b[i] != '`' {
                    w.push(b[i]);
                    i += 1;
                }
                if i < n {
                    w.push('`');
                    i += 1;
                }
                continue;
            }
            if c == '\\' && i + 1 < n {
                w.push(c);
                w.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == '$' && i + 1 < n && b[i + 1] == '(' {
                // $( ... ) or $(( ... )) — copy balanced
                let depth0 = w.len();
                let _ = depth0;
                w.push('$');
                w.push('(');
                i += 2;
                let mut depth = 1;
                while i < n && depth > 0 {
                    if b[i] == '(' {
                        depth += 1;
                    } else if b[i] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            w.push(')');
                            i += 1;
                            break;
                        }
                    }
                    w.push(b[i]);
                    i += 1;
                }
                continue;
            }
            if c == '$' && i + 1 < n && b[i + 1] == '{' {
                w.push('$');
                w.push('{');
                i += 2;
                while i < n && b[i] != '}' {
                    w.push(b[i]);
                    i += 1;
                }
                if i < n {
                    w.push('}');
                    i += 1;
                }
                continue;
            }
            w.push(c);
            i += 1;
        }
        if !w.is_empty() {
            toks.push(Tok::Word(w));
        } else if i < n && (b[i] == '(' || b[i] == ')') {
            // emit the paren operator we stopped on (handled at top of loop next)
        }
    }
    toks
}

// --- parser -------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Redir {
    Out(String),    // >
    Append(String), // >>
    In(String),     // <
    ErrOut(String), // 2>
    ErrToOut,       // 2>&1
}

#[derive(Clone, Debug)]
enum Node {
    Simple { assigns: Vec<(String, String)>, words: Vec<String>, redirs: Vec<Redir> },
    Pipeline(Vec<Node>),
    AndOr(Vec<(u8, Node)>), // 0=first, 1=&&, 2=||
    List(Vec<Node>),
    If { cond: alloc::boxed::Box<Node>, then: alloc::boxed::Box<Node>, elifs: Vec<(Node, Node)>, els: Option<alloc::boxed::Box<Node>> },
    For { var: String, words: Vec<String>, body: alloc::boxed::Box<Node> },
    While { cond: alloc::boxed::Box<Node>, body: alloc::boxed::Box<Node>, until: bool },
    Case { word: String, arms: Vec<(Vec<String>, Node)> },
    FuncDef { name: String, body: alloc::boxed::Box<Node> },
    Group(alloc::boxed::Box<Node>),
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn skip_seps(&mut self) {
        while matches!(self.peek(), Some(Tok::Newline) | Some(Tok::Op(";")) | Some(Tok::Op("&"))) {
            self.pos += 1;
        }
    }
    fn peek_word(&self) -> Option<&str> {
        match self.peek() {
            Some(Tok::Word(w)) => Some(w.as_str()),
            _ => None,
        }
    }
    /// Is the next token a reserved word that terminates a command list?
    fn at_terminator(&self) -> bool {
        matches!(
            self.peek_word(),
            Some("then" | "else" | "elif" | "fi" | "do" | "done" | "esac" | "}")
        ) || matches!(self.peek(), Some(Tok::Op(")")))
    }

    /// Parse a list of and-or pipelines until a terminator reserved word.
    fn parse_list(&mut self) -> Node {
        let mut items = Vec::new();
        loop {
            self.skip_seps();
            if self.at_end() || self.at_terminator() {
                break;
            }
            match self.parse_and_or() {
                Some(n) => items.push(n),
                None => break,
            }
        }
        if items.len() == 1 {
            items.pop().unwrap()
        } else {
            Node::List(items)
        }
    }

    fn parse_and_or(&mut self) -> Option<Node> {
        let first = self.parse_pipeline()?;
        let mut chain = vec![(0u8, first)];
        loop {
            match self.peek() {
                Some(Tok::Op("&&")) => {
                    self.pos += 1;
                    self.skip_newlines();
                    if let Some(p) = self.parse_pipeline() {
                        chain.push((1, p));
                    }
                }
                Some(Tok::Op("||")) => {
                    self.pos += 1;
                    self.skip_newlines();
                    if let Some(p) = self.parse_pipeline() {
                        chain.push((2, p));
                    }
                }
                _ => break,
            }
        }
        if chain.len() == 1 {
            Some(chain.pop().unwrap().1)
        } else {
            Some(Node::AndOr(chain))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Some(Tok::Newline)) {
            self.pos += 1;
        }
    }

    fn parse_pipeline(&mut self) -> Option<Node> {
        let mut stages = vec![self.parse_command()?];
        while matches!(self.peek(), Some(Tok::Op("|"))) {
            self.pos += 1;
            self.skip_newlines();
            stages.push(self.parse_command()?);
        }
        if stages.len() == 1 {
            Some(stages.pop().unwrap())
        } else {
            Some(Node::Pipeline(stages))
        }
    }

    fn parse_command(&mut self) -> Option<Node> {
        match self.peek_word() {
            Some("if") => return Some(self.parse_if()),
            Some("for") => return Some(self.parse_for()),
            Some("while") => return Some(self.parse_while(false)),
            Some("until") => return Some(self.parse_while(true)),
            Some("case") => return Some(self.parse_case()),
            Some("function") => return self.parse_func_kw(),
            _ => {}
        }
        if matches!(self.peek(), Some(Tok::Op("("))) {
            self.pos += 1;
            let body = self.parse_list();
            if matches!(self.peek(), Some(Tok::Op(")"))) {
                self.pos += 1;
            }
            return Some(Node::Group(alloc::boxed::Box::new(body)));
        }
        // function definition: name ( )
        if let Some(w) = self.peek_word() {
            if is_name(w)
                && matches!(self.toks.get(self.pos + 1), Some(Tok::Op("(")))
                && matches!(self.toks.get(self.pos + 2), Some(Tok::Op(")")))
            {
                let name = w.to_string();
                self.pos += 3;
                self.skip_newlines();
                // body is a brace group
                let body = self.parse_command()?;
                return Some(Node::FuncDef { name, body: alloc::boxed::Box::new(body) });
            }
            if w == "{" {
                self.pos += 1;
                let body = self.parse_list();
                if self.peek_word() == Some("}") {
                    self.pos += 1;
                }
                return Some(Node::Group(alloc::boxed::Box::new(body)));
            }
        }
        self.parse_simple()
    }

    fn parse_simple(&mut self) -> Option<Node> {
        let mut assigns = Vec::new();
        let mut words = Vec::new();
        let mut redirs = Vec::new();
        // leading VAR=value assignments
        while let Some(w) = self.peek_word() {
            if words.is_empty() && is_assignment(w) {
                let (k, v) = w.split_once('=').unwrap();
                assigns.push((k.to_string(), v.to_string()));
                self.pos += 1;
            } else {
                break;
            }
        }
        loop {
            match self.peek() {
                Some(Tok::Word(w)) => {
                    words.push(w.clone());
                    self.pos += 1;
                }
                Some(Tok::Op(">")) => {
                    self.pos += 1;
                    if let Some(t) = self.take_word() {
                        redirs.push(Redir::Out(t));
                    }
                }
                Some(Tok::Op(">>")) => {
                    self.pos += 1;
                    if let Some(t) = self.take_word() {
                        redirs.push(Redir::Append(t));
                    }
                }
                Some(Tok::Op("<")) => {
                    self.pos += 1;
                    if let Some(t) = self.take_word() {
                        redirs.push(Redir::In(t));
                    }
                }
                Some(Tok::Op("2>")) => {
                    self.pos += 1;
                    if let Some(t) = self.take_word() {
                        redirs.push(Redir::ErrOut(t));
                    }
                }
                Some(Tok::Op("2>&1")) => {
                    self.pos += 1;
                    redirs.push(Redir::ErrToOut);
                }
                _ => break,
            }
        }
        if assigns.is_empty() && words.is_empty() && redirs.is_empty() {
            return None;
        }
        Some(Node::Simple { assigns, words, redirs })
    }

    fn take_word(&mut self) -> Option<String> {
        if let Some(Tok::Word(w)) = self.peek() {
            let w = w.clone();
            self.pos += 1;
            Some(w)
        } else {
            None
        }
    }

    fn expect_word(&mut self, kw: &str) {
        if self.peek_word() == Some(kw) {
            self.pos += 1;
        }
    }

    fn parse_if(&mut self) -> Node {
        self.pos += 1; // if
        let cond = self.parse_list();
        self.expect_word("then");
        let then = self.parse_list();
        let mut elifs = Vec::new();
        let mut els = None;
        loop {
            match self.peek_word() {
                Some("elif") => {
                    self.pos += 1;
                    let c = self.parse_list();
                    self.expect_word("then");
                    let b = self.parse_list();
                    elifs.push((c, b));
                }
                Some("else") => {
                    self.pos += 1;
                    els = Some(alloc::boxed::Box::new(self.parse_list()));
                }
                _ => break,
            }
        }
        self.expect_word("fi");
        Node::If { cond: alloc::boxed::Box::new(cond), then: alloc::boxed::Box::new(then), elifs, els }
    }

    fn parse_for(&mut self) -> Node {
        self.pos += 1; // for
        let var = self.take_word().unwrap_or_default();
        let mut words = Vec::new();
        if self.peek_word() == Some("in") {
            self.pos += 1;
            while let Some(Tok::Word(w)) = self.peek() {
                words.push(w.clone());
                self.pos += 1;
            }
        } else {
            words.push("\"$@\"".to_string());
        }
        self.skip_seps();
        self.expect_word("do");
        let body = self.parse_list();
        self.expect_word("done");
        Node::For { var, words, body: alloc::boxed::Box::new(body) }
    }

    fn parse_while(&mut self, until: bool) -> Node {
        self.pos += 1; // while/until
        let cond = self.parse_list();
        self.expect_word("do");
        let body = self.parse_list();
        self.expect_word("done");
        Node::While { cond: alloc::boxed::Box::new(cond), body: alloc::boxed::Box::new(body), until }
    }

    fn parse_case(&mut self) -> Node {
        self.pos += 1; // case
        let word = self.take_word().unwrap_or_default();
        self.expect_word("in");
        let mut arms = Vec::new();
        loop {
            self.skip_seps();
            if self.peek_word() == Some("esac") || self.at_end() {
                break;
            }
            // optional leading '('
            if matches!(self.peek(), Some(Tok::Op("("))) {
                self.pos += 1;
            }
            // patterns separated by '|'
            let mut pats = Vec::new();
            loop {
                if let Some(w) = self.take_word() {
                    pats.push(w);
                }
                if matches!(self.peek(), Some(Tok::Op("|"))) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if matches!(self.peek(), Some(Tok::Op(")"))) {
                self.pos += 1;
            }
            let body = self.parse_list();
            arms.push((pats, body));
            if matches!(self.peek(), Some(Tok::Op(";;"))) {
                self.pos += 1;
            }
        }
        self.expect_word("esac");
        Node::Case { word, arms }
    }

    fn parse_func_kw(&mut self) -> Option<Node> {
        self.pos += 1; // function
        let name = self.take_word()?;
        if matches!(self.peek(), Some(Tok::Op("("))) {
            self.pos += 1;
            if matches!(self.peek(), Some(Tok::Op(")"))) {
                self.pos += 1;
            }
        }
        self.skip_newlines();
        let body = self.parse_command()?;
        Some(Node::FuncDef { name, body: alloc::boxed::Box::new(body) })
    }
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_assignment(s: &str) -> bool {
    if let Some(eq) = s.find('=') {
        eq > 0 && is_name(&s[..eq])
    } else {
        false
    }
}

// --- executor -----------------------------------------------------------------

const LOOP_CAP: usize = 100_000;

fn exec(node: &Node, st: &mut ShellState, stdin: Option<String>) -> (String, i32) {
    if st.exited {
        return (String::new(), st.status);
    }
    match node {
        Node::Simple { assigns, words, redirs } => exec_simple(assigns, words, redirs, st, stdin),
        Node::Pipeline(stages) => {
            let mut prev: Option<String> = stdin;
            let mut status = 0;
            for (i, s) in stages.iter().enumerate() {
                let (out, code) = exec(s, st, prev.take());
                status = code;
                if i + 1 < stages.len() {
                    prev = Some(out);
                } else {
                    return (out, status);
                }
            }
            (String::new(), status)
        }
        Node::AndOr(chain) => {
            let mut out = String::new();
            let mut status = 0;
            for (i, (conj, n)) in chain.iter().enumerate() {
                let run = match conj {
                    0 => true,
                    1 => status == 0, // &&
                    _ => status != 0, // ||
                };
                if i == 0 || run {
                    let (o, c) = exec(n, st, None);
                    out.push_str(&o);
                    status = c;
                }
            }
            (out, status)
        }
        Node::List(items) => {
            let mut out = String::new();
            let mut status = 0;
            for n in items {
                let (o, c) = exec(n, st, None);
                out.push_str(&o);
                status = c;
                if st.exited {
                    break;
                }
            }
            (out, status)
        }
        Node::Group(inner) => exec(inner, st, stdin),
        Node::If { cond, then, elifs, els } => {
            let (mut out, c) = exec(cond, st, None);
            if c == 0 {
                let (o, s) = exec(then, st, None);
                out.push_str(&o);
                return (out, s);
            }
            for (ec, eb) in elifs {
                let (o, cc) = exec(ec, st, None);
                out.push_str(&o);
                if cc == 0 {
                    let (o2, s) = exec(eb, st, None);
                    out.push_str(&o2);
                    return (out, s);
                }
            }
            if let Some(e) = els {
                let (o, s) = exec(e, st, None);
                out.push_str(&o);
                return (out, s);
            }
            (out, 0)
        }
        Node::For { var, words, body } => {
            let mut out = String::new();
            let mut status = 0;
            let mut items: Vec<String> = Vec::new();
            for w in words {
                items.extend(expand_word(w, st));
            }
            for item in items {
                st.vars.insert(var.clone(), item);
                let (o, s) = exec(body, st, None);
                out.push_str(&o);
                status = s;
                if st.exited {
                    break;
                }
            }
            (out, status)
        }
        Node::While { cond, body, until } => {
            let mut out = String::new();
            let mut status = 0;
            let mut guard = 0;
            loop {
                guard += 1;
                if guard > LOOP_CAP {
                    out.push_str("while: loop limit reached\n");
                    break;
                }
                let (co, cc) = exec(cond, st, None);
                out.push_str(&co);
                let go = if *until { cc != 0 } else { cc == 0 };
                if !go {
                    break;
                }
                let (o, s) = exec(body, st, None);
                out.push_str(&o);
                status = s;
                if st.exited {
                    break;
                }
            }
            (out, status)
        }
        Node::Case { word, arms } => {
            let target = expand_word(word, st).into_iter().next().unwrap_or_default();
            for (pats, body) in arms {
                for p in pats {
                    let pe = expand_word(p, st).into_iter().next().unwrap_or_else(|| p.clone());
                    if glob_match(&pe, &target) {
                        return exec(body, st, None);
                    }
                }
            }
            (String::new(), 0)
        }
        Node::FuncDef { name, body } => {
            st.funcs.insert(name.clone(), (**body).clone());
            (String::new(), 0)
        }
    }
}

fn exec_simple(
    assigns: &[(String, String)],
    words: &[String],
    redirs: &[Redir],
    st: &mut ShellState,
    stdin: Option<String>,
) -> (String, i32) {
    // Expand all words into the final argv.
    let mut argv: Vec<String> = Vec::new();
    for w in words {
        argv.extend(expand_word(w, st));
    }
    // Pure-assignment command (no words): set variables, status 0.
    if argv.is_empty() {
        for (k, v) in assigns {
            let val = expand_str(v, st);
            st.vars.insert(k.clone(), val);
        }
        return (String::new(), 0);
    }
    // Temporary assignment prefix (VAR=val cmd) — applied to the shell vars
    // (no separate environment fork in this model).
    for (k, v) in assigns {
        let val = expand_str(v, st);
        st.vars.insert(k.clone(), val);
    }

    // Input redirection / piped stdin.
    let mut input = stdin;
    let mut out_redir: Option<(String, bool)> = None; // (file, append)
    for r in redirs {
        match r {
            Redir::In(f) => {
                let f = expand_word(f, st).into_iter().next().unwrap_or_default();
                input = Some(fs::read_file(&f).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default());
            }
            Redir::Out(f) => out_redir = Some((expand_word(f, st).into_iter().next().unwrap_or_default(), false)),
            Redir::Append(f) => out_redir = Some((expand_word(f, st).into_iter().next().unwrap_or_default(), true)),
            Redir::ErrOut(_) | Redir::ErrToOut => {} // stderr merged with stdout in this model
        }
    }

    let (out, status) = dispatch(&argv, st, input);

    if let Some((file, append)) = out_redir {
        let res = if append {
            let mut prev = fs::read_file(&file).unwrap_or_default();
            prev.extend_from_slice(out.as_bytes());
            fs::write_file(&file, &prev)
        } else {
            fs::write_file(&file, out.as_bytes())
        };
        return match res {
            Ok(()) => (String::new(), status),
            Err(()) => (format!("{file}: write failed\n"), 1),
        };
    }
    (out, status)
}

/// Run one expanded command (builtin, function, or leaf file command).
fn dispatch(argv: &[String], st: &mut ShellState, stdin: Option<String>) -> (String, i32) {
    let name = argv[0].as_str();
    let rest: Vec<String> = argv[1..].to_vec();
    let args_joined = rest.join(" ");

    // user-defined function?
    if let Some(body) = st.funcs.get(name).cloned() {
        let saved = core::mem::replace(&mut st.params, rest.clone());
        let (o, s) = exec(&body, st, stdin);
        st.params = saved;
        return (o, s);
    }

    match name {
        "clear" => {
            st.clear = true;
            (String::new(), 0)
        }
        "exit" => {
            st.exited = true;
            (String::new(), rest.first().and_then(|s| s.parse().ok()).unwrap_or(0))
        }
        "true" | ":" => (String::new(), 0),
        "false" => (String::new(), 1),
        "echo" => echo(&rest),
        "printf" => (printf(&rest), 0),
        "pwd" => (format!("{}\n", st.vars.get("PWD").cloned().unwrap_or_else(|| "/".into())), 0),
        "cd" => (cd(&args_joined), 0),
        "export" => {
            for a in &rest {
                if let Some((k, v)) = a.split_once('=') {
                    st.vars.insert(k.to_string(), v.to_string());
                }
            }
            (String::new(), 0)
        }
        "unset" => {
            for a in &rest {
                st.vars.remove(a);
            }
            (String::new(), 0)
        }
        "set" => (String::new(), 0),
        "shift" => {
            if !st.params.is_empty() {
                st.params.remove(0);
            }
            (String::new(), 0)
        }
        "read" => {
            // read VAR -- consume the first line of stdin
            let line = stdin.as_deref().unwrap_or("").lines().next().unwrap_or("").to_string();
            if let Some(v) = rest.first() {
                st.vars.insert(v.clone(), line);
            }
            (String::new(), 0)
        }
        "let" => {
            let mut status = 1;
            for a in &rest {
                if let Some((k, e)) = a.split_once('=') {
                    let v = arith(e, st);
                    st.vars.insert(k.to_string(), v.to_string());
                    status = if v != 0 { 0 } else { 1 };
                } else {
                    let v = arith(a, st);
                    status = if v != 0 { 0 } else { 1 };
                }
            }
            (String::new(), status)
        }
        "test" | "[" | "[[" => {
            let mut a = rest.clone();
            if name != "test" {
                // drop trailing ] / ]]
                if a.last().map(|s| s == "]" || s == "]]").unwrap_or(false) {
                    a.pop();
                }
            }
            (String::new(), if test_eval(&a, st) { 0 } else { 1 })
        }
        "type" | "which" => {
            let q = rest.first().map(String::as_str).unwrap_or("");
            if st.funcs.contains_key(q) {
                (format!("{q} is a function\n"), 0)
            } else if is_builtin(q) {
                (format!("{q} is a shell builtin\n"), 0)
            } else if fs::read_file(&format!("{}.BIN", q.to_ascii_uppercase())).is_some() {
                (format!("{q} is /bin/{q}\n"), 0)
            } else {
                (format!("{q}: not found\n"), 1)
            }
        }
        "source" | "." => {
            let f = rest.first().cloned().unwrap_or_default();
            match fs::read_file(&f) {
                Some(d) => {
                    let body = String::from_utf8_lossy(&d).into_owned();
                    let toks = tokenize(&body);
                    let mut p = Parser { toks: &toks, pos: 0 };
                    let mut out = String::new();
                    let mut status = 0;
                    while !p.at_end() {
                        p.skip_seps();
                        if p.at_end() {
                            break;
                        }
                        if let Some(n) = p.parse_and_or() {
                            let (o, s) = exec(&n, st, None);
                            out.push_str(&o);
                            status = s;
                        } else {
                            p.pos += 1;
                        }
                        if st.exited {
                            break;
                        }
                    }
                    (out, status)
                }
                None => (format!("{f}: no such file\n"), 1),
            }
        }
        "sh" | "bash" => {
            // run a script file
            if let Some(f) = rest.first() {
                let mut a = alloc::vec!["source".to_string()];
                a.push(f.clone());
                return dispatch(&a, st, stdin);
            }
            (String::new(), 0)
        }
        "run" => {
            if let Some(app) = rest.first() {
                st.launch = Some(app.to_ascii_lowercase());
                (format!("launching {app}...\n"), 0)
            } else {
                ("run: missing app\n".to_string(), 1)
            }
        }
        "env" => (st.vars.iter().map(|(k, v)| format!("{k}={v}\n")).collect(), 0),
        "help" => (HELP.to_string(), 0),
        // --- leaf file/text commands ---
        "ls" => leaf(ls_router(&rest, &args_joined)),
        "cat" => leaf(cat_router(&rest, stdin.as_deref())),
        "mount" => (mount_cmd(&rest), 0),
        "umount" | "unmount" => (umount_cmd(&rest), 0),
        "pkg" => pkg_cmd(&rest),
        "cp" => leaf(cp(&args_joined)),
        "mv" => leaf(mv(&args_joined)),
        "rm" => leaf(rm(&args_joined)),
        "mkdir" => {
            // FAT16 here is root-only; accept (esp. `-p`) without failing a pipeline.
            let p = rest.iter().any(|a| a == "-p");
            if p { (String::new(), 0) } else { ("mkdir: FAT16 root-only (no subdirectories)\n".to_string(), 0) }
        }
        "touch" => {
            for f in &rest {
                if fs::read_file(f).is_none() {
                    let _ = fs::write_file(f, b"");
                }
            }
            (String::new(), 0)
        }
        "grep" | "egrep" => leaf(grep(&args_joined, stdin.as_deref())),
        "sed" => (sed(&rest, stdin.as_deref()), 0),
        "awk" => (awk(&rest, stdin.as_deref()), 0),
        "cut" => (cut(&rest, stdin.as_deref()), 0),
        "tr" => (tr(&rest, stdin.as_deref()), 0),
        "curl" | "wget" => curl(&rest),
        "head" => leaf(head(&args_joined, stdin.as_deref())),
        "tail" => leaf(tail(&args_joined, stdin.as_deref())),
        "sort" => leaf(sort(&args_joined, stdin.as_deref())),
        "uniq" => (uniq(&input_text(&args_joined, stdin.as_deref())), 0),
        "wc" => leaf(wc(&args_joined, stdin.as_deref())),
        "find" => leaf(find(&args_joined)),
        "date" => (date(), 0),
        "df" => (df(), 0),
        "nproc" => (format!("{}\n", crate::smp::nproc()), 0),
        "cc" => cc(&rest),
        "chmod" => (String::new(), 0),
        "seq" => (seq(&rest), 0),
        "basename" => (format!("{}\n", rest.first().map(|s| s.rsplit('/').next().unwrap_or(s)).unwrap_or("")), 0),
        "sleep" => (String::new(), 0),
        "jobs" => (String::new(), 0),
        "" => (String::new(), 0),
        other => (format!("{other}: command not found\n"), 127),
    }
}

/// `cc <file.c> [-o out.wsm]` — compile a C file to WASM inside Veil, write the
/// module to disk, and run it (printing its output). The on-OS C compiler.
fn cc(args: &[String]) -> (String, i32) {
    let src_name = args.iter().find(|a| !a.starts_with('-') && a.ends_with(".c")).cloned();
    let Some(src_name) = src_name else {
        return ("cc: usage: cc <file.c> [-o out.wsm]\n".to_string(), 2);
    };
    let Some(data) = fs::read_file(&src_name) else {
        return (format!("cc: {src_name}: no such file\n"), 1);
    };
    let src = String::from_utf8_lossy(&data);
    match crate::cc::compile(&src) {
        Ok(wasm) => {
            // Output name: -o NAME, else the source stem + .WSM.
            let out_name = args
                .windows(2)
                .find(|w| w[0] == "-o")
                .map(|w| w[1].clone())
                .unwrap_or_else(|| {
                    let stem: String = src_name.split('.').next().unwrap_or("A").chars().take(8).collect();
                    format!("{}.WSM", stem.to_ascii_uppercase())
                });
            let _ = fs::write_file(&out_name, &wasm);
            crate::kprintln!("CC: {src_name} -> {out_name} ({} bytes WASM)", wasm.len());
            // Run it and show the output.
            let mut out = format!("cc: compiled {} -> {} ({} bytes)\n", src_name, out_name, wasm.len());
            match crate::wasm::run(&wasm) {
                Ok(prog_out) => {
                    out.push_str(&prog_out);
                    if !prog_out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Err(e) => out.push_str(&format!("cc: run error: {e}\n")),
            }
            (out, 0)
        }
        Err(e) => (format!("cc: {src_name}: error: {e}\n"), 1),
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "echo" | "printf" | "cd" | "pwd" | "export" | "unset" | "set" | "shift" | "read" | "let"
            | "test" | "[" | "[[" | "type" | "which" | "source" | "." | "exit" | "true" | "false"
            | ":" | "help" | "env" | "run" | "sh" | "bash" | "clear"
    )
}

fn leaf(out: String) -> (String, i32) {
    let ok = !is_error(&out);
    (out, if ok { 0 } else { 1 })
}

fn echo(args: &[String]) -> (String, i32) {
    let mut a = args;
    let mut newline = true;
    let mut interpret = false;
    while let Some(first) = a.first() {
        match first.as_str() {
            "-n" => {
                newline = false;
                a = &a[1..];
            }
            "-e" => {
                interpret = true;
                a = &a[1..];
            }
            _ => break,
        }
    }
    let mut s = a.join(" ");
    if interpret {
        s = s.replace("\\n", "\n").replace("\\t", "\t");
    }
    if newline {
        s.push('\n');
    }
    (s, 0)
}

fn printf(args: &[String]) -> String {
    let Some(fmt) = args.first() else { return String::new() };
    let mut out = String::new();
    let mut ai = 1;
    let fmt = fmt.replace("\\n", "\n").replace("\\t", "\t");
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            let spec = chars[i + 1];
            match spec {
                's' => {
                    out.push_str(args.get(ai).map(String::as_str).unwrap_or(""));
                    ai += 1;
                }
                'd' | 'i' => {
                    let n: i64 = args.get(ai).and_then(|s| s.parse().ok()).unwrap_or(0);
                    out.push_str(&n.to_string());
                    ai += 1;
                }
                '%' => out.push('%'),
                _ => {
                    out.push('%');
                    out.push(spec);
                }
            }
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn cat_multi(files: &[String], stdin: Option<&str>) -> String {
    if files.is_empty() {
        return stdin.map(|s| {
            let mut s = s.to_string();
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s
        }).unwrap_or_default();
    }
    let mut out = String::new();
    for f in files {
        out.push_str(&cat(f));
    }
    out
}

/// `ls` that routes a path under a network mount to the remote server, else
/// falls back to the normal (FAT16) listing.
fn ls_router(rest: &[String], args_joined: &str) -> String {
    let target = rest.iter().find(|a| !a.starts_with('-')).cloned();
    if let Some(path) = target {
        let abspath = abspath(&path);
        if let Some((m, remote)) = crate::netfs::resolve_mount(&abspath) {
            return match crate::netfs::list_remote(m.ip, m.port, &remote) {
                crate::netfs::ListResult::Ok(entries) => {
                    let mut out = String::new();
                    for (name, is_dir, _sz) in entries {
                        out.push_str(&name);
                        if is_dir { out.push('/'); }
                        out.push('\n');
                    }
                    out
                }
                crate::netfs::ListResult::Err(e) => format!("ls: {path}: {e}\n"),
            };
        }
    }
    ls(args_joined)
}

/// `cat` that routes mounted paths to the remote server.
fn cat_router(files: &[String], stdin: Option<&str>) -> String {
    if files.is_empty() {
        return cat_multi(files, stdin);
    }
    let mut out = String::new();
    for f in files {
        let abspath = abspath(f);
        if let Some((m, remote)) = crate::netfs::resolve_mount(&abspath) {
            match crate::netfs::read_remote(m.ip, m.port, &remote) {
                Ok(data) => out.push_str(&String::from_utf8_lossy(&data)),
                Err(e) => out.push_str(&format!("cat: {f}: {e}\n")),
            }
        } else {
            out.push_str(&cat(f));
        }
    }
    out
}

/// Resolve a path against the VFS cwd into an absolute path (for mount routing).
fn abspath(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        let cwd = crate::vfs::get().cwd_path();
        if cwd == "/" { format!("/{path}") } else { format!("{cwd}/{path}") }
    }
}

/// `mount` (no args -> list mounts) / `mount <host:/path> <local>`.
fn mount_cmd(rest: &[String]) -> String {
    if rest.is_empty() {
        let mut out = String::new();
        for m in crate::netfs::list_mounts() {
            out.push_str(&format!("{}:{} on {} (netfs)\n", m.host, m.remote, m.local));
        }
        return out;
    }
    if rest.len() < 2 {
        return "mount: usage: mount <host:/path> <mountpoint>\n".to_string();
    }
    match crate::netfs::mount(&rest[0], &rest[1]) {
        Ok(()) => format!("mounted {} at {}\n", rest[0], rest[1]),
        Err(e) => format!("{e}\n"),
    }
}

/// `pkg install <name>` / `pkg remove <name>` / `pkg list` / `pkg update <name>`.
fn pkg_cmd(rest: &[String]) -> (String, i32) {
    let sub = rest.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "list" | "ls" => {
            let installed = crate::pkg::list_installed();
            if installed.is_empty() {
                return ("no packages installed\n".to_string(), 0);
            }
            let mut out = String::new();
            for (name, ver) in installed {
                out.push_str(&format!("{name} {ver}\n"));
            }
            (out, 0)
        }
        "install" | "add" | "update" | "upgrade" => {
            let Some(name) = rest.get(1) else {
                return ("pkg: usage: pkg install <name>\n".to_string(), 2);
            };
            if matches!(sub, "update" | "upgrade") {
                let _ = crate::pkg::remove(name); // reinstall on update
            }
            match crate::pkg::fetch_and_install(name) {
                Ok(n) => (format!("installed {n} (from {})\n", crate::pkg::REGISTRY), 0),
                Err(e) => (format!("{e}\n"), 1),
            }
        }
        "remove" | "rm" | "uninstall" => {
            let Some(name) = rest.get(1) else {
                return ("pkg: usage: pkg remove <name>\n".to_string(), 2);
            };
            match crate::pkg::remove(name) {
                Ok(()) => (format!("removed {name}\n"), 0),
                Err(e) => (format!("{e}\n"), 1),
            }
        }
        "" => ("pkg: usage: pkg <install|remove|list|update> [name]\n".to_string(), 2),
        other => (format!("pkg: unknown subcommand '{other}'\n"), 2),
    }
}

fn umount_cmd(rest: &[String]) -> String {
    let Some(local) = rest.first() else {
        return "umount: usage: umount <mountpoint>\n".to_string();
    };
    if crate::netfs::umount(local) {
        format!("unmounted {local}\n")
    } else {
        format!("umount: {local}: not mounted\n")
    }
}

fn uniq(text: &str) -> String {
    let mut out = String::new();
    let mut last: Option<&str> = None;
    for l in text.lines() {
        if Some(l) != last {
            out.push_str(l);
            out.push('\n');
            last = Some(l);
        }
    }
    out
}

fn seq(args: &[String]) -> String {
    let nums: Vec<i64> = args.iter().filter_map(|s| s.parse().ok()).collect();
    let (start, end, step) = match nums.len() {
        1 => (1, nums[0], 1),
        2 => (nums[0], nums[1], 1),
        3 => (nums[0], nums[2], nums[1]),
        _ => return String::new(),
    };
    let mut out = String::new();
    if step == 0 {
        return out;
    }
    let mut i = start;
    let mut guard = 0;
    while (step > 0 && i <= end) || (step < 0 && i >= end) {
        out.push_str(&i.to_string());
        out.push('\n');
        i += step;
        guard += 1;
        if guard > LOOP_CAP {
            break;
        }
    }
    out
}

// --- expansion ----------------------------------------------------------------

/// Expand one raw word into fields (after var/command/arith expansion, quote
/// removal, word splitting on unquoted whitespace, and globbing).
fn expand_word(raw: &str, st: &mut ShellState) -> Vec<String> {
    // Produce a string with markers for "no-split" (quoted) regions by tracking
    // splittability per char. Simpler approach: build the expanded string and a
    // parallel "quoted" bitmap, then split on unquoted whitespace.
    let (s, quoted) = expand_marked(raw, st);
    // word-split on unquoted whitespace
    let chars: Vec<char> = s.chars().collect();
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    for (i, &c) in chars.iter().enumerate() {
        let q = quoted.get(i).copied().unwrap_or(false);
        if !q && (c == ' ' || c == '\t' || c == '\n') {
            if started {
                fields.push(core::mem::take(&mut cur));
                started = false;
            }
        } else {
            cur.push(c);
            started = true;
        }
    }
    if started {
        fields.push(cur);
    }
    // glob each unquoted field; if it has glob chars, expand against the disk
    let mut out = Vec::new();
    for f in fields {
        if f.contains('*') || f.contains('?') || f.contains('[') {
            let matches = glob_expand(&f);
            if matches.is_empty() {
                out.push(f);
            } else {
                out.extend(matches);
            }
        } else {
            out.push(f);
        }
    }
    if out.is_empty() && raw.is_empty() {
        out.push(String::new());
    }
    out
}

/// Expand a raw word producing (text, per-char quoted flag).
fn expand_marked(raw: &str, st: &mut ShellState) -> (String, Vec<bool>) {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut quoted: Vec<bool> = Vec::new();
    let mut i = 0;
    let n = chars.len();
    let mut push = |out: &mut String, quoted: &mut Vec<bool>, s: &str, q: bool| {
        for ch in s.chars() {
            out.push(ch);
            quoted.push(q);
        }
    };
    while i < n {
        let c = chars[i];
        if c == '\'' {
            i += 1;
            while i < n && chars[i] != '\'' {
                push(&mut out, &mut quoted, &chars[i].to_string(), true);
                i += 1;
            }
            i += 1; // closing '
            continue;
        }
        if c == '"' {
            i += 1;
            while i < n && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < n && matches!(chars[i + 1], '"' | '\\' | '$' | '`') {
                    push(&mut out, &mut quoted, &chars[i + 1].to_string(), true);
                    i += 2;
                    continue;
                }
                if chars[i] == '$' {
                    let (val, ni) = expand_dollar(&chars, i, st);
                    push(&mut out, &mut quoted, &val, true);
                    i = ni;
                    continue;
                }
                if chars[i] == '`' {
                    let (val, ni) = expand_backtick(&chars, i, st);
                    push(&mut out, &mut quoted, &val, true);
                    i = ni;
                    continue;
                }
                push(&mut out, &mut quoted, &chars[i].to_string(), true);
                i += 1;
            }
            i += 1; // closing "
            continue;
        }
        if c == '\\' && i + 1 < n {
            push(&mut out, &mut quoted, &chars[i + 1].to_string(), true);
            i += 2;
            continue;
        }
        if c == '$' {
            let (val, ni) = expand_dollar(&chars, i, st);
            push(&mut out, &mut quoted, &val, false);
            i = ni;
            continue;
        }
        if c == '`' {
            let (val, ni) = expand_backtick(&chars, i, st);
            push(&mut out, &mut quoted, &val, false);
            i = ni;
            continue;
        }
        if c == '~' && (i == 0) && (i + 1 >= n || chars[i + 1] == '/') {
            push(&mut out, &mut quoted, "/", false);
            i += 1;
            continue;
        }
        push(&mut out, &mut quoted, &c.to_string(), false);
        i += 1;
    }
    (out, quoted)
}

/// Expand a string fully (no field-splitting), e.g. an assignment RHS.
fn expand_str(raw: &str, st: &mut ShellState) -> String {
    expand_marked(raw, st).0
}

/// Expand a `$...` starting at index `i`; returns (value, next_index).
fn expand_dollar(chars: &[char], i: usize, st: &mut ShellState) -> (String, usize) {
    let n = chars.len();
    // i points at '$'
    if i + 1 >= n {
        return ("$".to_string(), i + 1);
    }
    let c = chars[i + 1];
    if c == '(' {
        // $(( arith )) or $( cmd )
        if i + 2 < n && chars[i + 2] == '(' {
            // arithmetic
            let mut depth = 0;
            let mut j = i + 1;
            let start = i + 3;
            while j < n {
                if chars[j] == '(' {
                    depth += 1;
                } else if chars[j] == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            // j is at the outer ')'; arithmetic content is start..j-1 (strip inner ')')
            let endexpr = j.saturating_sub(1);
            let expr: String = chars[start..endexpr.min(n)].iter().collect();
            let v = arith(&expr, st);
            let next = (j + 1).min(n);
            return (v.to_string(), next);
        }
        // command substitution $( ... )
        let mut depth = 1;
        let mut j = i + 2;
        let start = j;
        while j < n && depth > 0 {
            if chars[j] == '(' {
                depth += 1;
            } else if chars[j] == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        let cmd: String = chars[start..j.min(n)].iter().collect();
        let out = run_capture(&cmd, st);
        return (out, (j + 1).min(n));
    }
    if c == '{' {
        let mut j = i + 2;
        while j < n && chars[j] != '}' {
            j += 1;
        }
        let inner: String = chars[i + 2..j.min(n)].iter().collect();
        let val = expand_braced_var(&inner, st);
        return (val, (j + 1).min(n));
    }
    if c == '?' {
        return (st.status.to_string(), i + 2);
    }
    if c == '#' {
        return (st.params.len().to_string(), i + 2);
    }
    if c == '@' || c == '*' {
        return (st.params.join(" "), i + 2);
    }
    if c == '$' {
        return ("vsh".to_string(), i + 2); // $$ pid placeholder
    }
    if c.is_ascii_digit() {
        let idx = c.to_digit(10).unwrap() as usize;
        let val = if idx == 0 { "vsh".to_string() } else { st.params.get(idx - 1).cloned().unwrap_or_default() };
        return (val, i + 2);
    }
    if c.is_ascii_alphabetic() || c == '_' {
        let mut j = i + 1;
        while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
            j += 1;
        }
        let name: String = chars[i + 1..j].iter().collect();
        return (st.vars.get(&name).cloned().unwrap_or_default(), j);
    }
    ("$".to_string(), i + 1)
}

fn expand_backtick(chars: &[char], i: usize, st: &mut ShellState) -> (String, usize) {
    let n = chars.len();
    let mut j = i + 1;
    while j < n && chars[j] != '`' {
        j += 1;
    }
    let cmd: String = chars[i + 1..j.min(n)].iter().collect();
    let out = run_capture(&cmd, st);
    (out, (j + 1).min(n))
}

/// `${VAR}`, `${VAR:-default}`, `${VAR:=default}`, `${#VAR}`.
fn expand_braced_var(inner: &str, st: &mut ShellState) -> String {
    if let Some(name) = inner.strip_prefix('#') {
        return st.vars.get(name).map(|v| v.len()).unwrap_or(0).to_string();
    }
    if let Some((name, def)) = inner.split_once(":-") {
        let cur = lookup(name, st);
        return if cur.is_empty() { expand_str(def, st) } else { cur };
    }
    if let Some((name, def)) = inner.split_once(":=") {
        let cur = lookup(name, st);
        if cur.is_empty() {
            let v = expand_str(def, st);
            st.vars.insert(name.to_string(), v.clone());
            return v;
        }
        return cur;
    }
    lookup(inner, st)
}

fn lookup(name: &str, st: &ShellState) -> String {
    match name {
        "?" => st.status.to_string(),
        "#" => st.params.len().to_string(),
        "@" | "*" => st.params.join(" "),
        _ => {
            if let Ok(idx) = name.parse::<usize>() {
                if idx == 0 {
                    return "vsh".to_string();
                }
                return st.params.get(idx - 1).cloned().unwrap_or_default();
            }
            st.vars.get(name).cloned().unwrap_or_default()
        }
    }
}

/// Run a command line and capture its stdout (trailing newlines trimmed).
fn run_capture(cmd: &str, st: &mut ShellState) -> String {
    let toks = tokenize(cmd);
    let mut p = Parser { toks: &toks, pos: 0 };
    let mut out = String::new();
    while !p.at_end() {
        p.skip_seps();
        if p.at_end() {
            break;
        }
        if let Some(n) = p.parse_and_or() {
            let (o, s) = exec(&n, st, None);
            out.push_str(&o);
            st.status = s;
        } else {
            p.pos += 1;
        }
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

// --- arithmetic ---------------------------------------------------------------

fn arith(expr: &str, st: &mut ShellState) -> i64 {
    let s = expand_str(expr, st);
    let toks = arith_tokens(&s, st);
    let mut pos = 0;
    arith_expr(&toks, &mut pos)
}

#[derive(Clone)]
enum ATok {
    Num(i64),
    Op(char),
    Op2([char; 2]),
    Open,
    Close,
}

fn arith_tokens(s: &str, st: &ShellState) -> Vec<ATok> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let n = chars.len();
    while i < n {
        let c = chars[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let mut num = 0i64;
            while i < n && chars[i].is_ascii_digit() {
                num = num * 10 + chars[i].to_digit(10).unwrap() as i64;
                i += 1;
            }
            out.push(ATok::Num(num));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut name = String::new();
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            let v = st.vars.get(&name).and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0);
            out.push(ATok::Num(v));
            continue;
        }
        if c == '(' {
            out.push(ATok::Open);
            i += 1;
            continue;
        }
        if c == ')' {
            out.push(ATok::Close);
            i += 1;
            continue;
        }
        // two-char comparators
        if i + 1 < n && matches!((c, chars[i + 1]), ('=', '=') | ('!', '=') | ('<', '=') | ('>', '=') | ('&', '&') | ('|', '|')) {
            out.push(ATok::Op2([c, chars[i + 1]]));
            i += 2;
            continue;
        }
        out.push(ATok::Op(c));
        i += 1;
    }
    out
}

fn arith_expr(t: &[ATok], pos: &mut usize) -> i64 {
    arith_cmp(t, pos)
}

fn arith_cmp(t: &[ATok], pos: &mut usize) -> i64 {
    let mut acc = arith_add(t, pos);
    while let Some(tok) = t.get(*pos) {
        match tok {
            ATok::Op2(['=', '=']) => { *pos += 1; let r = arith_add(t, pos); acc = (acc == r) as i64; }
            ATok::Op2(['!', '=']) => { *pos += 1; let r = arith_add(t, pos); acc = (acc != r) as i64; }
            ATok::Op2(['<', '=']) => { *pos += 1; let r = arith_add(t, pos); acc = (acc <= r) as i64; }
            ATok::Op2(['>', '=']) => { *pos += 1; let r = arith_add(t, pos); acc = (acc >= r) as i64; }
            ATok::Op2(['&', '&']) => { *pos += 1; let r = arith_add(t, pos); acc = ((acc != 0) && (r != 0)) as i64; }
            ATok::Op2(['|', '|']) => { *pos += 1; let r = arith_add(t, pos); acc = ((acc != 0) || (r != 0)) as i64; }
            ATok::Op('<') => { *pos += 1; let r = arith_add(t, pos); acc = (acc < r) as i64; }
            ATok::Op('>') => { *pos += 1; let r = arith_add(t, pos); acc = (acc > r) as i64; }
            _ => break,
        }
    }
    acc
}

fn arith_add(t: &[ATok], pos: &mut usize) -> i64 {
    let mut acc = arith_mul(t, pos);
    while let Some(ATok::Op(op @ ('+' | '-'))) = t.get(*pos) {
        let op = *op;
        *pos += 1;
        let r = arith_mul(t, pos);
        acc = if op == '+' { acc + r } else { acc - r };
    }
    acc
}

fn arith_mul(t: &[ATok], pos: &mut usize) -> i64 {
    let mut acc = arith_unary(t, pos);
    while let Some(ATok::Op(op @ ('*' | '/' | '%'))) = t.get(*pos) {
        let op = *op;
        *pos += 1;
        let r = arith_unary(t, pos);
        acc = match op {
            '*' => acc * r,
            '/' => if r != 0 { acc / r } else { 0 },
            _ => if r != 0 { acc % r } else { 0 },
        };
    }
    acc
}

fn arith_unary(t: &[ATok], pos: &mut usize) -> i64 {
    match t.get(*pos) {
        Some(ATok::Op('-')) => { *pos += 1; -arith_unary(t, pos) }
        Some(ATok::Op('+')) => { *pos += 1; arith_unary(t, pos) }
        Some(ATok::Op('!')) => { *pos += 1; (arith_unary(t, pos) == 0) as i64 }
        Some(ATok::Num(n)) => { let v = *n; *pos += 1; v }
        Some(ATok::Open) => {
            *pos += 1;
            let v = arith_cmp(t, pos);
            if matches!(t.get(*pos), Some(ATok::Close)) {
                *pos += 1;
            }
            v
        }
        _ => 0,
    }
}

// --- test / [ ] ---------------------------------------------------------------

fn test_eval(args: &[String], st: &ShellState) -> bool {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    test_or(&a)
}

fn test_or(a: &[&str]) -> bool {
    // split on -o
    if let Some(p) = a.iter().position(|&x| x == "-o") {
        return test_and(&a[..p]) || test_or(&a[p + 1..]);
    }
    test_and(a)
}

fn test_and(a: &[&str]) -> bool {
    if let Some(p) = a.iter().position(|&x| x == "-a") {
        return test_prim(&a[..p]) && test_and(&a[p + 1..]);
    }
    test_prim(a)
}

fn test_prim(a: &[&str]) -> bool {
    match a.len() {
        0 => false,
        1 => !a[0].is_empty(),
        2 => {
            let neg = a[0] == "!";
            if neg {
                return !test_prim(&a[1..]);
            }
            match a[0] {
                "-e" => fs::read_file(a[1]).is_some() || file_exists(a[1]),
                "-f" => file_exists(a[1]),
                "-d" => a[1] == "/" || a[1] == ".",
                "-s" => fs::read_file(a[1]).map(|d| !d.is_empty()).unwrap_or(false),
                "-z" => a[1].is_empty(),
                "-n" => !a[1].is_empty(),
                _ => false,
            }
        }
        3 => {
            let (l, op, r) = (a[0], a[1], a[2]);
            match op {
                "=" | "==" => glob_match(r, l) || l == r,
                "!=" => l != r,
                "-eq" => num(l) == num(r),
                "-ne" => num(l) != num(r),
                "-lt" => num(l) < num(r),
                "-le" => num(l) <= num(r),
                "-gt" => num(l) > num(r),
                "-ge" => num(l) >= num(r),
                _ => false,
            }
        }
        _ => {
            if a[0] == "!" {
                return !test_prim(&a[1..]);
            }
            // chained: evaluate first 3 then ignore (best effort)
            test_prim(&a[..3])
        }
    }
}

fn num(s: &str) -> i64 {
    s.trim().parse().unwrap_or(0)
}

fn file_exists(name: &str) -> bool {
    let up = name.trim_start_matches('/').to_ascii_uppercase();
    fs::list_root().unwrap_or_default().iter().any(|(n, _)| n.eq_ignore_ascii_case(&up))
        || fs::read_file(name).is_some()
}

// --- glob ---------------------------------------------------------------------

/// Match a glob pattern against a literal string (case-insensitive for FAT16).
fn glob_match(pat: &str, s: &str) -> bool {
    glob_rec(&pat.chars().collect::<Vec<_>>(), &s.chars().collect::<Vec<_>>())
}

fn glob_rec(p: &[char], s: &[char]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }
    match p[0] {
        '*' => {
            for k in 0..=s.len() {
                if glob_rec(&p[1..], &s[k..]) {
                    return true;
                }
            }
            false
        }
        '?' => !s.is_empty() && glob_rec(&p[1..], &s[1..]),
        '[' => {
            // [abc] / [a-z]
            if let Some(close) = p.iter().position(|&c| c == ']') {
                if s.is_empty() {
                    return false;
                }
                let set = &p[1..close];
                if char_in_set(set, s[0]) {
                    return glob_rec(&p[close + 1..], &s[1..]);
                }
            }
            false
        }
        c => !s.is_empty() && eqc(c, s[0]) && glob_rec(&p[1..], &s[1..]),
    }
}

fn eqc(a: char, b: char) -> bool {
    a.eq_ignore_ascii_case(&b)
}

fn char_in_set(set: &[char], c: char) -> bool {
    let mut i = 0;
    while i < set.len() {
        if i + 2 < set.len() && set[i + 1] == '-' {
            if c >= set[i] && c <= set[i + 2] {
                return true;
            }
            i += 3;
        } else {
            if eqc(set[i], c) {
                return true;
            }
            i += 1;
        }
    }
    false
}

/// Expand a glob field against the FAT16 root listing (sorted).
fn glob_expand(field: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::list_root().unwrap_or_default().into_iter().map(|(n, _)| n).collect();
    names.sort();
    names.into_iter().filter(|n| glob_match(field, n)).collect()
}

// --- tiny regex (grep/awk): . ^ $ * + ? [..] \x ----------------------------

enum ReAtom {
    Ch(char),
    Any,
    Class(Vec<(char, char)>, bool),
}
enum ReQ {
    One,
    Star,
    Plus,
    Opt,
}

struct Regex {
    atoms: Vec<(ReAtom, ReQ)>,
    anchor_start: bool,
    anchor_end: bool,
}

fn re_compile(pat: &str) -> Regex {
    let chars: Vec<char> = pat.chars().collect();
    let mut atoms = Vec::new();
    let mut i = 0;
    let n = chars.len();
    let anchor_start = chars.first() == Some(&'^');
    if anchor_start {
        i = 1;
    }
    let mut anchor_end = false;
    while i < n {
        let c = chars[i];
        if c == '$' && i + 1 == n {
            anchor_end = true;
            i += 1;
            break;
        }
        let atom = match c {
            '.' => {
                i += 1;
                ReAtom::Any
            }
            '\\' if i + 1 < n => {
                i += 2;
                ReAtom::Ch(chars[i - 1])
            }
            '[' => {
                let mut j = i + 1;
                let neg = j < n && chars[j] == '^';
                if neg {
                    j += 1;
                }
                let mut ranges = Vec::new();
                while j < n && chars[j] != ']' {
                    if j + 2 < n && chars[j + 1] == '-' && chars[j + 2] != ']' {
                        ranges.push((chars[j], chars[j + 2]));
                        j += 3;
                    } else {
                        ranges.push((chars[j], chars[j]));
                        j += 1;
                    }
                }
                i = if j < n { j + 1 } else { j };
                ReAtom::Class(ranges, neg)
            }
            _ => {
                i += 1;
                ReAtom::Ch(c)
            }
        };
        let q = match chars.get(i) {
            Some('*') => {
                i += 1;
                ReQ::Star
            }
            Some('+') => {
                i += 1;
                ReQ::Plus
            }
            Some('?') => {
                i += 1;
                ReQ::Opt
            }
            _ => ReQ::One,
        };
        atoms.push((atom, q));
    }
    Regex { atoms, anchor_start, anchor_end }
}

fn atom_matches(a: &ReAtom, c: char) -> bool {
    match a {
        ReAtom::Any => c != '\n',
        ReAtom::Ch(x) => *x == c,
        ReAtom::Class(ranges, neg) => {
            let hit = ranges.iter().any(|(lo, hi)| c >= *lo && c <= *hi);
            hit != *neg
        }
    }
}

fn re_match_here(re: &Regex, ai: usize, s: &[char], si: usize) -> bool {
    if ai == re.atoms.len() {
        return !re.anchor_end || si == s.len();
    }
    let (atom, q) = &re.atoms[ai];
    match q {
        ReQ::One => si < s.len() && atom_matches(atom, s[si]) && re_match_here(re, ai + 1, s, si + 1),
        ReQ::Opt => {
            (si < s.len() && atom_matches(atom, s[si]) && re_match_here(re, ai + 1, s, si + 1))
                || re_match_here(re, ai + 1, s, si)
        }
        ReQ::Star | ReQ::Plus => {
            // count maximal run, backtrack down to the minimum
            let mut k = si;
            while k < s.len() && atom_matches(atom, s[k]) {
                k += 1;
            }
            let min = if matches!(q, ReQ::Plus) { si + 1 } else { si };
            let mut j = k;
            while j + 1 > min {
                if re_match_here(re, ai + 1, s, j) {
                    return true;
                }
                if j == 0 {
                    break;
                }
                j -= 1;
            }
            j >= min && re_match_here(re, ai + 1, s, j)
        }
    }
}

/// Does `pat` match anywhere in `line` (regex)?
fn re_search(pat: &str, line: &str) -> bool {
    let re = re_compile(pat);
    let s: Vec<char> = line.chars().collect();
    if re.anchor_start {
        return re_match_here(&re, 0, &s, 0);
    }
    for start in 0..=s.len() {
        if re_match_here(&re, 0, &s, start) {
            return true;
        }
    }
    false
}

// --- sed / awk / cut / tr / curl ---------------------------------------------

fn sed(args: &[String], stdin: Option<&str>) -> String {
    // sed [-n] 's/old/new/[g]'  |  sed '/pat/d'  (over stdin or a file)
    let mut script = String::new();
    let mut file = String::new();
    for tok in args {
        if tok.starts_with('-') {
            continue;
        } else if script.is_empty() {
            script = tok.clone();
        } else {
            file = tok.clone();
        }
    }
    let text = input_text(&file, stdin);
    let scr = script.trim().trim_matches('\'').trim_matches('"');
    if let Some(rest) = scr.strip_prefix('s') {
        let delim = rest.chars().next().unwrap_or('/');
        let parts: Vec<&str> = rest[delim.len_utf8()..].split(delim).collect();
        if parts.len() >= 2 {
            let (old, new) = (parts[0], parts[1]);
            let global = parts.get(2).map(|f| f.contains('g')).unwrap_or(false);
            let mut out = String::new();
            for l in text.lines() {
                let r = if global { l.replace(old, new) } else { replace_first(l, old, new) };
                out.push_str(&r);
                out.push('\n');
            }
            return out;
        }
    }
    if let Some(p) = scr.strip_suffix('d').map(str::trim) {
        let pat = p.trim_matches('/');
        return text.lines().filter(|l| !re_search(pat, l)).map(|l| format!("{l}\n")).collect();
    }
    text
}

fn replace_first(s: &str, old: &str, new: &str) -> String {
    if let Some(i) = s.find(old) {
        let mut r = String::from(&s[..i]);
        r.push_str(new);
        r.push_str(&s[i + old.len()..]);
        r
    } else {
        s.to_string()
    }
}

fn awk(args: &[String], stdin: Option<&str>) -> String {
    // awk [-F<sep>] '<pattern> { print <items> }'  (subset)
    let mut sep: Option<String> = None;
    let mut prog = String::new();
    let mut file = String::new();
    let mut it = args.iter().cloned();
    while let Some(t) = it.next() {
        if let Some(f) = t.strip_prefix("-F") {
            sep = Some(if f.is_empty() { it.next().unwrap_or_default() } else { f.to_string() });
        } else if prog.is_empty() {
            prog = t;
        } else {
            file = t;
        }
    }
    let prog = prog.trim().trim_matches('\'').trim_matches('"').to_string();
    // optional leading /regex/ pattern, then { action }
    let (pattern, action) = if let Some(open) = prog.find('{') {
        let pat = prog[..open].trim().to_string();
        let act = prog[open + 1..].trim_end().trim_end_matches('}').trim().to_string();
        (pat, act)
    } else {
        (prog.clone(), String::from("print"))
    };
    let re_pat = pattern.trim().strip_prefix('/').and_then(|p| p.strip_suffix('/')).map(String::from);
    let text = input_text(&file, stdin);
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        let nr = i + 1;
        let fields: Vec<&str> = match &sep {
            Some(s) => line.split(s.as_str()).collect(),
            None => line.split_whitespace().collect(),
        };
        // pattern: /re/ or NR==k or empty
        let matched = if let Some(p) = &re_pat {
            re_search(p, line)
        } else if let Some(eq) = pattern.strip_prefix("NR==") {
            eq.trim().parse::<usize>().map(|k| k == nr).unwrap_or(false)
        } else {
            true
        };
        if !matched {
            continue;
        }
        out.push_str(&awk_action(&action, line, &fields, nr));
        out.push('\n');
    }
    out
}

fn awk_action(action: &str, line: &str, fields: &[&str], nr: usize) -> String {
    let act = action.trim();
    let body = act.strip_prefix("print").map(str::trim).unwrap_or(act);
    if body.is_empty() {
        return line.to_string();
    }
    let mut parts = Vec::new();
    for item in body.split(',') {
        let item = item.trim();
        if item == "NR" {
            parts.push(nr.to_string());
        } else if item == "NF" {
            parts.push(fields.len().to_string());
        } else if item == "$0" {
            parts.push(line.to_string());
        } else if let Some(n) = item.strip_prefix('$') {
            if let Ok(k) = n.trim().parse::<usize>() {
                parts.push(fields.get(k.saturating_sub(1)).copied().unwrap_or("").to_string());
            }
        } else {
            parts.push(item.trim_matches('"').to_string());
        }
    }
    parts.join(" ")
}

fn cut(args: &[String], stdin: Option<&str>) -> String {
    let mut delim = '\t';
    let mut fields_spec = String::new();
    let mut chars_spec = String::new();
    let mut file = String::new();
    let mut it = args.iter().cloned();
    while let Some(t) = it.next() {
        if let Some(d) = t.strip_prefix("-d") {
            let ds = if d.is_empty() { it.next().unwrap_or_default() } else { d.to_string() };
            delim = ds.trim_matches('\'').chars().next().unwrap_or('\t');
        } else if let Some(f) = t.strip_prefix("-f") {
            fields_spec = if f.is_empty() { it.next().unwrap_or_default() } else { f.to_string() };
        } else if let Some(c) = t.strip_prefix("-c") {
            chars_spec = if c.is_empty() { it.next().unwrap_or_default() } else { c.to_string() };
        } else {
            file = t;
        }
    }
    let text = input_text(&file, stdin);
    let mut out = String::new();
    for line in text.lines() {
        if !chars_spec.is_empty() {
            let chars: Vec<char> = line.chars().collect();
            let sel = parse_range(&chars_spec, chars.len());
            let s: String = sel.iter().filter_map(|&i| chars.get(i - 1)).collect();
            out.push_str(&s);
        } else {
            let parts: Vec<&str> = line.split(delim).collect();
            let sel = parse_range(&fields_spec, parts.len());
            let s: Vec<&str> = sel.iter().filter_map(|&i| parts.get(i - 1).copied()).collect();
            out.push_str(&s.join(&delim.to_string()));
        }
        out.push('\n');
    }
    out
}

/// Parse a cut range list like "1,3", "2-", "1-5" into 1-based indices.
fn parse_range(spec: &str, max: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        if let Some((a, b)) = part.split_once('-') {
            let lo: usize = a.trim().parse().unwrap_or(1);
            let hi: usize = if b.trim().is_empty() { max } else { b.trim().parse().unwrap_or(max) };
            for i in lo..=hi.min(max) {
                out.push(i);
            }
        } else if let Ok(i) = part.trim().parse::<usize>() {
            out.push(i);
        }
    }
    out
}

fn tr(args: &[String], stdin: Option<&str>) -> String {
    let mut del = false;
    let mut sets: Vec<String> = Vec::new();
    for t in args {
        if t == "-d" {
            del = true;
        } else if t.starts_with('-') && t.len() > 1 && t.chars().nth(1) == Some('d') {
            del = true;
        } else {
            sets.push(t.trim_matches('\'').trim_matches('"').to_string());
        }
    }
    let text = input_text("", stdin);
    if del {
        let set: Vec<char> = expand_set(sets.first().map(String::as_str).unwrap_or(""));
        return text.chars().filter(|c| !set.contains(c)).collect();
    }
    let from = expand_set(sets.first().map(String::as_str).unwrap_or(""));
    let to = expand_set(sets.get(1).map(String::as_str).unwrap_or(""));
    text.chars()
        .map(|c| match from.iter().position(|&x| x == c) {
            Some(i) => to.get(i.min(to.len().saturating_sub(1))).copied().unwrap_or(c),
            None => c,
        })
        .collect()
}

/// Expand a tr set like "a-z" or "A-Z0-9" into a char list.
fn expand_set(s: &str) -> Vec<char> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' {
            for c in chars[i]..=chars[i + 2] {
                out.push(c);
            }
            i += 3;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn curl(args: &[String]) -> (String, i32) {
    let mut url = String::new();
    let mut out_file: Option<String> = None;
    let mut post_data: Option<String> = None;
    let mut silent = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-s" | "-S" | "-L" | "-k" => silent = a == "-s",
            "-o" => out_file = it.next().cloned(),
            "-d" | "--data" => post_data = it.next().cloned(),
            "-X" => {
                it.next();
            }
            _ if !a.starts_with('-') => url = a.clone(),
            _ => {}
        }
    }
    let _ = silent;
    if url.is_empty() {
        return ("curl: no URL\n".to_string(), 2);
    }
    let body = post_data.as_ref().map(|s| s.as_bytes());
    match crate::browser::shell_fetch(&url, body) {
        Some((status, data)) => {
            crate::kprintln!("CURL: {url} -> {status} ({} bytes)", data.len());
            let text = String::from_utf8_lossy(&data).into_owned();
            if let Some(f) = out_file {
                return match fs::write_file(&f, &data) {
                    Ok(()) => (String::new(), 0),
                    Err(()) => (format!("curl: cannot write {f}\n"), 1),
                };
            }
            let mut t = text;
            if !t.ends_with('\n') {
                t.push('\n');
            }
            (t, if status == 200 { 0 } else { 1 })
        }
        None => (format!("curl: ({url}) could not fetch (no network?)\n"), 7),
    }
}

/// Split a raw args string into whitespace-separated tokens, honoring quotes.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = '\0';
    let mut started = false;
    for c in s.chars() {
        if quote != '\0' {
            if c == quote {
                quote = '\0';
            } else {
                cur.push(c);
            }
            started = true;
        } else if c == '\'' || c == '"' {
            quote = c;
            started = true;
        } else if c.is_whitespace() {
            if started {
                out.push(core::mem::take(&mut cur));
                started = false;
            }
        } else {
            cur.push(c);
            started = true;
        }
    }
    if started {
        out.push(cur);
    }
    out
}

// --- leaf file/text commands (kept from the M35 shell) ------------------------

fn is_error(out: &str) -> bool {
    out.lines().any(|l| {
        l.contains(": no such")
            || l.contains(": not found")
            || l.contains("command not found")
            || l.contains(": missing")
            || l.contains(": write failed")
            || l == "ls: no filesystem"
    })
}

fn ls(args: &str) -> String {
    let long = args.split_whitespace().any(|a| a == "-l");
    let Some(mut files) = fs::list_root() else {
        return "ls: no filesystem\n".to_string();
    };
    files.sort();
    let mut out = String::new();
    for (name, size) in files {
        if long {
            out.push_str(&format!("{size:>8}  {name}\n"));
        } else {
            out.push_str(&format!("{name}\n"));
        }
    }
    out
}

fn cat(args: &str) -> String {
    if args.is_empty() {
        return "cat: missing file\n".to_string();
    }
    match fs::read_file(args.trim()) {
        Some(data) => {
            let mut s = String::from_utf8_lossy(&data).into_owned();
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s
        }
        None => format!("cat: {}: no such file\n", args.trim()),
    }
}

fn cp(args: &str) -> String {
    let Some((src, dst)) = args.split_once(char::is_whitespace) else {
        return "usage: cp <src> <dst>\n".to_string();
    };
    let (src, dst) = (src.trim(), dst.trim());
    let Some(data) = fs::read_file(src) else {
        return format!("cp: {src}: no such file\n");
    };
    match fs::write_file(dst, &data) {
        Ok(()) => String::new(),
        Err(()) => format!("cp: {dst}: write failed\n"),
    }
}

fn mv(args: &str) -> String {
    let Some((src, dst)) = args.split_once(char::is_whitespace) else {
        return "usage: mv <src> <dst>\n".to_string();
    };
    let (src, dst) = (src.trim(), dst.trim());
    let Some(data) = fs::read_file(src) else {
        return format!("mv: {src}: no such file\n");
    };
    if fs::write_file(dst, &data).is_err() {
        return format!("mv: {dst}: write failed\n");
    }
    let _ = fs::delete(src);
    String::new()
}

fn rm(args: &str) -> String {
    if args.is_empty() {
        return "rm: missing file\n".to_string();
    }
    let mut out = String::new();
    for f in args.split_whitespace().filter(|a| !a.starts_with('-')) {
        if fs::delete(f).is_err() {
            out.push_str(&format!("rm: {f}: no such file\n"));
        }
    }
    out
}

fn cd(args: &str) -> String {
    match args.trim() {
        "" | "/" | "." | "~" => String::new(),
        other => format!("cd: {other}: root-only filesystem\n"),
    }
}

fn input_text(arg: &str, stdin: Option<&str>) -> String {
    if let Some(s) = stdin {
        return s.to_string();
    }
    let f = arg.split_whitespace().last().unwrap_or("");
    if !f.is_empty() {
        if let Some(d) = fs::read_file(f) {
            return String::from_utf8_lossy(&d).into_owned();
        }
    }
    String::new()
}

fn grep(args: &str, stdin: Option<&str>) -> String {
    // flags: -i -v -n -c -l -r (regex pattern by default)
    let (mut ci, mut inv, mut numbered, mut count, mut list, mut recurse) =
        (false, false, false, false, false, false);
    let mut rest = args;
    loop {
        let r = rest.trim_start();
        if let Some(f) = r.strip_prefix('-') {
            let flag: String = f.chars().take_while(|c| !c.is_whitespace()).collect();
            if flag.is_empty() || !flag.chars().all(|c| "ivnclr".contains(c)) {
                break;
            }
            for ch in flag.chars() {
                match ch {
                    'i' => ci = true,
                    'v' => inv = true,
                    'n' => numbered = true,
                    'c' => count = true,
                    'l' => list = true,
                    'r' => recurse = true,
                    _ => {}
                }
            }
            rest = &r[1 + flag.len()..];
        } else {
            break;
        }
    }
    let (pat0, file) = rest.trim().split_once(char::is_whitespace).unwrap_or((rest.trim(), ""));
    let pat0 = pat0.trim().trim_matches('"').trim_matches('\'');
    let pat = if ci { pat0.to_ascii_lowercase() } else { pat0.to_string() };
    let hit = |l: &str| {
        let target = if ci { l.to_ascii_lowercase() } else { l.to_string() };
        re_search(&pat, &target) != inv
    };
    // -r: scan every root file, prefix matches with "name:".
    if recurse {
        let mut out = String::new();
        for (name, _) in fs::list_root().unwrap_or_default() {
            let data = fs::read_file(&name).unwrap_or_default();
            let text = String::from_utf8_lossy(&data);
            let mut any = false;
            for l in text.lines() {
                if hit(l) {
                    any = true;
                    if !list {
                        out.push_str(&format!("{name}:{l}\n"));
                    }
                }
            }
            if list && any {
                out.push_str(&format!("{name}\n"));
            }
        }
        return out;
    }
    let text = input_text(file, stdin);
    if count {
        return format!("{}\n", text.lines().filter(|l| hit(l)).count());
    }
    if list {
        return if text.lines().any(hit) && !file.is_empty() { format!("{file}\n") } else { String::new() };
    }
    let mut out = String::new();
    for (i, l) in text.lines().enumerate() {
        if hit(l) {
            if numbered {
                out.push_str(&format!("{}:{l}\n", i + 1));
            } else {
                out.push_str(&format!("{l}\n"));
            }
        }
    }
    out
}

fn nlines(args: &str) -> (usize, &str) {
    let a = args.trim();
    if let Some(r) = a.strip_prefix("-n") {
        let r = r.trim_start();
        let n: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = r[n.len()..].trim_start();
        (n.parse().unwrap_or(10), rest)
    } else {
        (10, a)
    }
}

fn head(args: &str, stdin: Option<&str>) -> String {
    let (n, file) = nlines(args);
    input_text(file, stdin).lines().take(n).map(|l| format!("{l}\n")).collect()
}

fn tail(args: &str, stdin: Option<&str>) -> String {
    let (n, file) = nlines(args);
    let text = input_text(file, stdin);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| format!("{l}\n")).collect()
}

fn sort(args: &str, stdin: Option<&str>) -> String {
    let reverse = args.split_whitespace().any(|a| a == "-r");
    let numeric = args.split_whitespace().any(|a| a == "-n");
    let file = args.split_whitespace().filter(|a| !a.starts_with('-')).last().unwrap_or("");
    let text = input_text(file, stdin);
    let mut lines: Vec<&str> = text.lines().collect();
    if numeric {
        lines.sort_by_key(|l| l.trim().parse::<i64>().unwrap_or(0));
    } else {
        lines.sort();
    }
    if reverse {
        lines.reverse();
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn wc(args: &str, stdin: Option<&str>) -> String {
    let flag = args.split_whitespace().next().filter(|a| a.starts_with('-')).unwrap_or("");
    let file = args.split_whitespace().filter(|a| !a.starts_with('-')).last().unwrap_or("");
    let s = input_text(file, stdin);
    let (l, w, c) = (s.lines().count(), s.split_whitespace().count(), s.len());
    match flag {
        "-l" => format!("{l}\n"),
        "-w" => format!("{w}\n"),
        "-c" => format!("{c}\n"),
        _ => format!("{l} {w} {c}\n"),
    }
}

fn find(args: &str) -> String {
    let pat = args.split_once("-name").map(|(_, p)| p.trim()).unwrap_or("*");
    let mut out = String::new();
    for (name, _) in fs::list_root().unwrap_or_default() {
        if glob_match(pat, &name) || pat == "*" {
            out.push_str(&format!("/{name}\n"));
        }
    }
    out
}

fn date() -> String {
    let unix = crate::timer::wall_ticks50().map(|t| t / 50);
    match unix {
        Some(s) => format!("{}\n", civil(s as i64)),
        None => format!("uptime {}s (no NTP sync)\n", crate::timer::uptime_secs()),
    }
}

fn civil(t: i64) -> String {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", y, m, d, secs / 3600, (secs / 60) % 60, secs % 60)
}

fn df() -> String {
    let files = fs::list_root().unwrap_or_default();
    let used: u32 = files.iter().map(|(_, s)| s).sum();
    format!("Filesystem  Used  Files\nFAT16       {} bytes  {}\n", used, files.len())
}

/// Boot self-test: exercise the interpreter's control flow, expansion,
/// arithmetic, functions and pipes (no filesystem needed). Emits SHELL_OK.
pub fn selftest() {
    let script = "\
x=5\n\
y=3\n\
arith=$((x * y + 1))\n\
greet() { echo \"hi $1\"; }\n\
acc=\"\"\n\
for i in 1 2 3; do acc=\"$acc$i\"; done\n\
if [ \"$arith\" -eq 16 ]; then cls=big; else cls=small; fi\n\
n=0\n\
while [ $n -lt 3 ]; do n=$((n + 1)); done\n\
case $cls in\n\
  big) tag=BIG ;;\n\
  *) tag=other ;;\n\
esac\n\
sub=$(echo nested | grep nest)\n\
echo \"arith=$arith acc=$acc cls=$cls n=$n tag=$tag $(greet veil) sub=$sub\"\n";
    let out = run(script).out;
    crate::kprintln!("SHELL_SELFTEST: {}", out.trim());
    let ok = out.contains("arith=16")
        && out.contains("acc=123")
        && out.contains("cls=big")
        && out.contains("n=3")
        && out.contains("tag=BIG")
        && out.contains("hi veil")
        && out.contains("sub=nested");
    if ok {
        crate::kprintln!("SHELL_OK: vars/arith/for/while/if/case/functions/pipes/cmd-subst all work");
    } else {
        crate::kprintln!("SHELL_FAIL: {}", out.trim());
    }
    // Tidy the test's leftovers out of the interactive shell's state.
    let st = state();
    for k in ["x", "y", "arith", "acc", "i", "cls", "n", "tag", "sub"] {
        st.vars.remove(k);
    }
    st.funcs.remove("greet");
}

/// Coreutils self-test: grep (regex/anchors/classes), sed, awk, cut, tr through
/// pipes and command substitution. Emits COREUTILS_OK.
pub fn coreutils_selftest() {
    let script = r#"
r1=$(echo apple | tr a-z A-Z)
r2=$(echo a:b:c | cut -d: -f2)
r3=$(echo "x 10 y" | awk '{print $2}')
r4=$(echo foofoo | sed 's/foo/bar/g')
r5=$(echo hello42 | grep "[0-9][0-9]")
r6=$(echo END | grep "ND$")
r7=$(echo "p,1
q,2
p,3" | awk -F, '/^p/{print $2}' | sort -r | head -1)
echo "r1=$r1 r2=$r2 r3=$r3 r4=$r4 r5=$r5 r6=$r6 r7=$r7"
"#;
    let out = run(script).out;
    crate::kprintln!("COREUTILS: {}", out.trim());
    let ok = out.contains("r1=APPLE")
        && out.contains("r2=b")
        && out.contains("r3=10")
        && out.contains("r4=barbar")
        && out.contains("r5=hello42")
        && out.contains("r6=END")
        && out.contains("r7=3");
    if ok {
        crate::kprintln!("COREUTILS_OK: grep(regex) sed awk cut tr through pipes all work");
    } else {
        crate::kprintln!("COREUTILS_FAIL: {}", out.trim());
    }
    let st = state();
    for k in ["r1", "r2", "r3", "r4", "r5", "r6", "r7"] {
        st.vars.remove(k);
    }
}

/// Filenames on disk, for tab completion.
pub fn complete(prefix: &str) -> Vec<String> {
    let prefix_up = prefix.to_ascii_uppercase();
    fs::list_root()
        .unwrap_or_default()
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n.starts_with(&prefix_up))
        .collect()
}
