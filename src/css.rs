//! M16/M34 CSS parser for the browser's documented subset.
//!
//! Selectors: compound selectors of `tag`, `.class`, `tag.class`, joined by
//! the descendant (` `) or child (`>`, treated as descendant) combinator, in
//! comma-separated groups — so `.nav-links a`, `.about-body p`, `article.post`
//! all work. Elements with several classes match a rule naming any one of them.
//! Anything fancier (ids, pseudo-classes/elements, attribute or sibling
//! selectors, universal) makes that whole selector skipped.
//!
//! `@media` / `@keyframes` / `@font-face` / `@supports` blocks are skipped
//! entirely: their contents are mobile overrides, animation keyframes or
//! font declarations that would otherwise leak into the (desktop) render.
//!
//! Declarations are kept as raw (property, value) strings; the browser's style
//! resolver interprets the supported properties and ignores the rest.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;

static DARK_DONE: AtomicBool = AtomicBool::new(false);

/// True for `@media (prefers-color-scheme: dark)` (and conjunctions of it that
/// don't also gate on a mobile max-width). Such blocks hold a site's dark theme,
/// which matches Veil's dark desktop, so we apply them instead of skipping.
fn is_dark_media(prelude: &str) -> bool {
    let p = prelude.to_ascii_lowercase();
    p.starts_with("@media")
        && p.contains("prefers-color-scheme")
        && p.contains("dark")
        && !p.contains("light")
        && !p.contains("max-width")
}

/// One compound selector: an optional tag and an optional single class.
pub struct Sel {
    pub tag: Option<String>,
    pub class: Option<String>,
}

impl Sel {
    /// Parse one compound (`tag`, `.class`, `tag.class`). Returns None for any
    /// unsupported token so the caller can drop the whole selector.
    fn parse(s: &str) -> Option<Sel> {
        let s = s.trim();
        if s.is_empty()
            || s.contains(|c: char| {
                matches!(c, '#' | ':' | '[' | ']' | '*' | '+' | '~' | '(' | ')')
            })
        {
            return None;
        }
        let (tag, class) = match s.split_once('.') {
            // `.a.b` (multiple classes in one compound) is unsupported.
            Some((_, c)) if c.contains('.') => return None,
            Some((t, c)) => ((!t.is_empty()).then(|| t.to_ascii_lowercase()), Some(String::from(c))),
            None => (Some(s.to_ascii_lowercase()), None),
        };
        (tag.is_some() || class.is_some()).then_some(Sel { tag, class })
    }

    /// Does this compound match an element with `tag` and the given (raw,
    /// space-separated) class attribute?
    fn matches(&self, tag: &str, class_attr: Option<&str>) -> bool {
        if let Some(t) = &self.tag {
            if t != tag {
                return false;
            }
        }
        if let Some(c) = &self.class {
            let hit = class_attr.is_some_and(|ca| ca.split_whitespace().any(|x| x == c));
            if !hit {
                return false;
            }
        }
        true
    }
}

pub struct Rule {
    pub key: Sel,            // rightmost compound — must match the element itself
    pub ancestors: Vec<Sel>, // descendant requirements (each must match an ancestor)
    pub decls: Vec<(String, String)>,
}

impl Rule {
    /// Crude specificity: number of classes across key + ancestors. Higher wins,
    /// so `.nav-links a` (rank 1) beats `a` (rank 0).
    pub fn rank(&self) -> u32 {
        self.ancestors.iter().filter(|s| s.class.is_some()).count() as u32
            + self.key.class.is_some() as u32
    }

    /// Match the element `(tag, class_attr)` whose ancestor chain (root..parent)
    /// is `anc`, each entry being `(tag, class_attr)`.
    pub fn matches(&self, tag: &str, class_attr: Option<&str>, anc: &[(&str, Option<&str>)]) -> bool {
        if !self.key.matches(tag, class_attr) {
            return false;
        }
        // Every required ancestor must match somewhere up the chain (descendant
        // combinator — not necessarily a direct parent).
        self.ancestors
            .iter()
            .all(|need| anc.iter().any(|(t, c)| need.matches(t, *c)))
    }
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("*/") {
            Some(end) => &rest[start + end + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn parse_decls(body: &str) -> Vec<(String, String)> {
    body.split(';')
        .filter_map(|d| {
            let (p, v) = d.split_once(':')?;
            let (p, v) = (p.trim(), v.trim());
            (!p.is_empty() && !v.is_empty()).then(|| (p.to_ascii_lowercase(), String::from(v)))
        })
        .collect()
}

/// Parse a full selector (one comma group) into a Rule (no decls yet). The
/// child combinator `>` is treated as a descendant. Returns None if any
/// compound is unsupported.
fn parse_selector(sel: &str) -> Option<Rule> {
    let norm = sel.replace('>', " ");
    let mut compounds: Vec<Sel> = Vec::new();
    for part in norm.split_whitespace() {
        compounds.push(Sel::parse(part)?);
    }
    let key = compounds.pop()?;
    Some(Rule { key, ancestors: compounds, decls: Vec::new() })
}

/// Collect all CSS custom-property declarations (`--name: value`) from every
/// rule block, regardless of selector (so `:root { --x: ... }`, which `parse`
/// skips as an unsupported selector, still contributes its variables). Later
/// declarations win, so callers should resolve a name to its LAST entry.
pub fn collect_vars(src: &str) -> Vec<(String, String)> {
    let src = strip_comments(src);
    let mut vars = Vec::new();
    let mut rest = src.as_str();
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else {
            break;
        };
        for d in rest[open + 1..close].split(';') {
            if let Some((p, v)) = d.split_once(':') {
                let (p, v) = (p.trim(), v.trim());
                if p.starts_with("--") && !v.is_empty() {
                    vars.push((String::from(p), String::from(v)));
                }
            }
        }
        rest = &rest[close + 1..];
    }
    vars
}

pub fn parse(src: &str) -> Vec<Rule> {
    let src = strip_comments(src);
    let b = src.as_bytes();
    let mut rules = Vec::new();
    let mut i = 0;
    let mut start = 0; // start of the current selector prelude
    while i < b.len() {
        match b[i] {
            // A top-level `;` ends an at-statement (e.g. `@import url(...);`).
            b';' => {
                start = i + 1;
                i += 1;
            }
            b'{' => {
                let prelude = src[start..i].trim();
                if prelude.starts_with('@') {
                    // @media / @keyframes / @font-face / @supports: skip the
                    // whole (possibly nested) block — EXCEPT a dark color-scheme
                    // media query, whose rules we DO apply (Veil renders dark).
                    let mut depth = 0;
                    let mut j = i;
                    while j < b.len() {
                        match b[j] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    j += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if is_dark_media(prelude) {
                        // Inner content is between the outer braces (i+1 .. j-1).
                        let inner = &src[i + 1..j.saturating_sub(1).min(src.len())];
                        rules.extend(parse(inner));
                        if !DARK_DONE.swap(true, core::sync::atomic::Ordering::Relaxed) {
                            crate::kprintln!("CSS_DARK_OK");
                        }
                    }
                    i = j;
                    start = j;
                } else {
                    let mut j = i + 1;
                    while j < b.len() && b[j] != b'}' {
                        j += 1;
                    }
                    let decls = parse_decls(&src[i + 1..j.min(src.len())]);
                    for sel in prelude.split(',') {
                        if let Some(mut rule) = parse_selector(sel) {
                            rule.decls = decls.clone();
                            rules.push(rule);
                        }
                    }
                    i = (j + 1).min(b.len());
                    start = i;
                }
            }
            b'}' => {
                start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    rules
}
