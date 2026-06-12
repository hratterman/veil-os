//! JavaScript tokenizer. Handles identifiers/keywords, numbers, single/double
//! quoted strings, template literals (with `${}` interpolation pre-tokenized),
//! line/block comments, and the operator/punctuation subset the engine needs.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub enum Tok {
    Num(f64),
    Str(String),
    /// Template literal split into string chunks and pre-tokenized expressions.
    Tmpl(Vec<TplPart>),
    Ident(String),
    /// Operator/punctuation, e.g. "==", "=>", "...", "+".
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub enum TplPart {
    Str(String),
    Expr(Vec<Tok>),
}

const KEYWORDS: &[&str] = &[
    "var", "let", "const", "function", "return", "if", "else", "for", "while", "do", "break",
    "continue", "true", "false", "null", "undefined", "new", "typeof", "this", "of", "in",
    "instanceof", "void", "delete", "switch", "case", "default", "throw", "try", "catch", "finally",
];

pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

struct Lexer<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
}

pub fn tokenize(src: &str) -> Vec<Tok> {
    let mut lx = Lexer { b: src.as_bytes(), s: src, i: 0 };
    let mut out = Vec::new();
    lx.run(&mut out, None);
    out.push(Tok::Eof);
    out
}

impl<'a> Lexer<'a> {
    /// Tokenize until EOF, or (for template `${}`) until an unmatched `}`.
    fn run(&mut self, out: &mut Vec<Tok>, stop_brace: Option<()>) {
        let mut depth = 0i32;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            // whitespace
            if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
                self.i += 1;
                continue;
            }
            // comments
            if c == b'/' && self.peek(1) == Some(b'/') {
                while self.i < self.b.len() && self.b[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            if c == b'/' && self.peek(1) == Some(b'*') {
                self.i += 2;
                while self.i + 1 < self.b.len() && !(self.b[self.i] == b'*' && self.b[self.i + 1] == b'/') {
                    self.i += 1;
                }
                self.i += 2;
                continue;
            }
            // template literal
            if c == b'`' {
                out.push(self.template());
                continue;
            }
            // strings
            if c == b'"' || c == b'\'' {
                out.push(Tok::Str(self.string(c)));
                continue;
            }
            // numbers
            if c.is_ascii_digit() || (c == b'.' && self.peek(1).map(|d| d.is_ascii_digit()).unwrap_or(false)) {
                out.push(self.number());
                continue;
            }
            // identifiers / keywords
            if c == b'_' || c == b'$' || c.is_ascii_alphabetic() {
                out.push(Tok::Ident(self.ident()));
                continue;
            }
            // brace tracking for template `${}` termination
            if stop_brace.is_some() {
                if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        self.i += 1; // consume the closing }
                        return;
                    }
                    depth -= 1;
                }
            }
            // operators / punctuation
            if let Some(p) = self.punct() {
                out.push(Tok::Punct(p));
                continue;
            }
            // unknown byte: skip
            self.i += 1;
        }
    }

    fn peek(&self, n: usize) -> Option<u8> {
        self.b.get(self.i + n).copied()
    }

    fn string(&mut self, quote: u8) -> String {
        self.i += 1;
        let mut s = String::new();
        while self.i < self.b.len() && self.b[self.i] != quote {
            let c = self.b[self.i];
            if c == b'\\' {
                self.i += 1;
                if self.i >= self.b.len() {
                    break;
                }
                s.push(unescape(self.b[self.i]));
                self.i += 1;
            } else {
                let ch_len = self.s[self.i..].chars().next().map_or(1, |c| c.len_utf8());
                s.push_str(&self.s[self.i..self.i + ch_len]);
                self.i += ch_len;
            }
        }
        self.i += 1; // closing quote
        s
    }

    fn template(&mut self) -> Tok {
        self.i += 1; // opening backtick
        let mut parts: Vec<TplPart> = Vec::new();
        let mut cur = String::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'`' {
                self.i += 1;
                break;
            }
            if c == b'\\' {
                self.i += 1;
                if self.i < self.b.len() {
                    cur.push(unescape(self.b[self.i]));
                    self.i += 1;
                }
                continue;
            }
            if c == b'$' && self.peek(1) == Some(b'{') {
                if !cur.is_empty() {
                    parts.push(TplPart::Str(core::mem::take(&mut cur)));
                }
                self.i += 2; // past ${
                let mut sub = Vec::new();
                self.run(&mut sub, Some(())); // consumes through the matching }
                sub.push(Tok::Eof);
                parts.push(TplPart::Expr(sub));
                continue;
            }
            let ch_len = self.s[self.i..].chars().next().map_or(1, |c| c.len_utf8());
            cur.push_str(&self.s[self.i..self.i + ch_len]);
            self.i += ch_len;
        }
        if !cur.is_empty() {
            parts.push(TplPart::Str(cur));
        }
        Tok::Tmpl(parts)
    }

    fn number(&mut self) -> Tok {
        let start = self.i;
        if self.b[self.i] == b'0' && matches!(self.peek(1), Some(b'x') | Some(b'X')) {
            self.i += 2;
            while self.i < self.b.len() && self.b[self.i].is_ascii_hexdigit() {
                self.i += 1;
            }
            let n = i64::from_str_radix(&self.s[start + 2..self.i], 16).unwrap_or(0);
            return Tok::Num(n as f64);
        }
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_digit() || self.b[self.i] == b'.' || self.b[self.i] == b'e' || self.b[self.i] == b'E'
                || ((self.b[self.i] == b'+' || self.b[self.i] == b'-') && matches!(self.b.get(self.i - 1), Some(b'e') | Some(b'E'))))
        {
            self.i += 1;
        }
        Tok::Num(self.s[start..self.i].parse::<f64>().unwrap_or(0.0))
    }

    fn ident(&mut self) -> String {
        let start = self.i;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'_' || c == b'$' || c.is_ascii_alphanumeric() {
                self.i += 1;
            } else {
                break;
            }
        }
        String::from(&self.s[start..self.i])
    }

    fn punct(&mut self) -> Option<&'static str> {
        // Longest-match against known operators.
        const THREE: &[&str] = &["===", "!==", "...", ">>>", "&&=", "||=", "**="];
        const TWO: &[&str] = &[
            "==", "!=", "<=", ">=", "&&", "||", "=>", "++", "--", "+=", "-=", "*=", "/=", "%=", "?.",
            "??", "**", "<<", ">>",
        ];
        const ONE: &[&str] = &[
            "+", "-", "*", "/", "%", "=", "<", ">", "!", "?", ":", ".", ",", ";", "(", ")", "[", "]",
            "{", "}", "&", "|", "^", "~",
        ];
        let rest = &self.s[self.i..];
        for p in THREE {
            if rest.starts_with(p) {
                self.i += 3;
                return Some(p);
            }
        }
        for p in TWO {
            if rest.starts_with(p) {
                self.i += 2;
                return Some(p);
            }
        }
        for p in ONE {
            if rest.starts_with(p) {
                self.i += 1;
                return Some(p);
            }
        }
        None
    }
}

fn unescape(c: u8) -> char {
    match c {
        b'n' => '\n',
        b't' => '\t',
        b'r' => '\r',
        b'0' => '\0',
        b'\\' => '\\',
        b'`' => '`',
        _ => c as char,
    }
}
