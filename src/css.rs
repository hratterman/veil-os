//! M16 CSS parser: a flat rule list for the browser's documented subset.
//! Selectors: `tag`, `.class`, `tag.class`, comma-separated groups.
//! Anything fancier (descendants, ids, pseudo-classes) is skipped whole.
//! Declarations are kept as raw (property, value) strings; the browser's
//! style resolver interprets the supported properties and ignores the
//! rest.

use alloc::string::String;
use alloc::vec::Vec;

pub struct Rule {
    pub tag: Option<String>,
    pub class: Option<String>,
    pub decls: Vec<(String, String)>,
}

impl Rule {
    /// Specificity rank: tag = 0, .class / tag.class = 1.
    pub fn rank(&self) -> u32 {
        self.class.is_some() as u32
    }

    pub fn matches(&self, tag: &str, class: Option<&str>) -> bool {
        if let Some(t) = &self.tag {
            if t != tag {
                return false;
            }
        }
        if let Some(c) = &self.class {
            if class != Some(c.as_str()) {
                return false;
            }
        }
        true
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
    let mut rules = Vec::new();
    let mut rest = src.as_str();
    while let Some(open) = rest.find('{') {
        let selectors = rest[..open].trim();
        let Some(close) = rest[open..].find('}').map(|c| open + c) else {
            break;
        };
        let body = &rest[open + 1..close];
        let decls: Vec<(String, String)> = body
            .split(';')
            .filter_map(|d| {
                let (p, v) = d.split_once(':')?;
                let (p, v) = (p.trim(), v.trim());
                (!p.is_empty() && !v.is_empty())
                    .then(|| (p.to_ascii_lowercase(), String::from(v)))
            })
            .collect();
        for sel in selectors.split(',') {
            let sel = sel.trim();
            // Only the documented forms; skip anything more structured.
            if sel.is_empty()
                || sel.contains(|c: char| c.is_ascii_whitespace() || c == '#' || c == ':' || c == '>')
            {
                continue;
            }
            let (tag, class) = match sel.split_once('.') {
                Some((t, c)) => (
                    (!t.is_empty()).then(|| t.to_ascii_lowercase()),
                    Some(String::from(c)),
                ),
                None => (Some(sel.to_ascii_lowercase()), None),
            };
            if tag.is_none() && class.is_none() {
                continue;
            }
            rules.push(Rule { tag, class, decls: decls.clone() });
        }
        rest = &rest[close + 1..];
    }
    rules
}
