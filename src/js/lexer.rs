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
    /// Regex literal: (pattern, flags). e.g. /ab+c/gi.
    Regex(String, String),
    /// Operator/punctuation, e.g. "==", "=>", "...", "+".
    Punct(&'static str),
    Eof,
}

/// A `/` starts a regex literal (rather than division) when the previous token
/// is not a value — i.e. at expression start, after an operator/punctuator, or
/// after a keyword like `return`/`typeof`. After a value (ident, number, string,
/// `)`, `]`), `/` is the division operator.
fn regex_allowed(prev: Option<&Tok>) -> bool {
    match prev {
        None => true,
        Some(Tok::Num(_)) | Some(Tok::Str(_)) | Some(Tok::Tmpl(_)) | Some(Tok::Regex(..)) => false,
        Some(Tok::Ident(s)) => matches!(
            s.as_str(),
            "return" | "typeof" | "instanceof" | "in" | "of" | "do" | "else" | "void"
                | "delete" | "new" | "case" | "throw" | "yield" | "await"
        ),
        Some(Tok::Punct(p)) => !matches!(*p, ")" | "]" | "}"),
        Some(Tok::Eof) => true,
    }
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
            // regex literal (decided by the preceding token; falls through to
            // the `/` division operator if no closing slash is found)
            if c == b'/' && regex_allowed(out.last()) {
                let save = self.i;
                if let Some((pat, flags)) = self.regex() {
                    out.push(Tok::Regex(pat, flags));
                    continue;
                }
                self.i = save;
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

    /// Scan a regex literal starting at the opening `/`. Returns (pattern, flags)
    /// or None if no closing `/` is found on the line (so the caller falls back
    /// to treating `/` as division). Honours `\` escapes and `[...]` classes
    /// (where `/` is literal).
    fn regex(&mut self) -> Option<(String, String)> {
        debug_assert_eq!(self.b[self.i], b'/');
        let mut j = self.i + 1;
        let mut pat = String::new();
        let mut in_class = false;
        while j < self.b.len() {
            let c = self.b[j];
            if c == b'\n' {
                return None; // unterminated on this line -> not a regex
            }
            if c == b'\\' {
                // keep the escape verbatim (it's part of the pattern)
                if j + 1 < self.b.len() {
                    pat.push('\\');
                    pat.push(self.b[j + 1] as char);
                    j += 2;
                    continue;
                }
                return None;
            }
            if c == b'[' {
                in_class = true;
            } else if c == b']' {
                in_class = false;
            } else if c == b'/' && !in_class {
                // end of pattern
                j += 1;
                let mut flags = String::new();
                while j < self.b.len() && self.b[j].is_ascii_alphabetic() {
                    flags.push(self.b[j] as char);
                    j += 1;
                }
                self.i = j;
                return Some((pat, flags));
            }
            // copy the byte (regex patterns are ASCII-dominant; copy raw)
            pat.push(c as char);
            j += 1;
        }
        None
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
        // Longest-match against known operators (4 chars first).
        const FOUR: &[&str] = &[">>>="];
        const THREE: &[&str] = &["===", "!==", "...", ">>>", "&&=", "||=", "**=", "<<=", ">>=", "??="];
        const TWO: &[&str] = &[
            "==", "!=", "<=", ">=", "&&", "||", "=>", "++", "--", "+=", "-=", "*=", "/=", "%=", "?.",
            "??", "**", "<<", ">>", "|=", "&=", "^=",
        ];
        const ONE: &[&str] = &[
            "+", "-", "*", "/", "%", "=", "<", ">", "!", "?", ":", ".", ",", ";", "(", ")", "[", "]",
            "{", "}", "&", "|", "^", "~",
        ];
        let rest = &self.s[self.i..];
        for (n, set) in [(4usize, FOUR), (3, THREE), (2, TWO), (1, ONE)] {
            for p in set {
                if rest.starts_with(p) {
                    self.i += n;
                    return Some(p);
                }
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
