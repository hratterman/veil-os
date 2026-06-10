//! M16 HTML parser: tokenizer + tree builder for the browser's documented
//! subset (see src/browser.rs for the exact grammar). Stack-based, with
//! the small amount of error tolerance the subset needs: void elements
//! (br, img, link, meta, hr), self-closing slashes, `<li>`/`<p>` implicit
//! sibling close, stray close tags ignored, comments and doctypes skipped.
//! Unknown tags become generic inline containers.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub enum Node {
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    },
    Text(String),
}

impl Node {
    pub fn tag(&self) -> Option<&str> {
        match self {
            Node::Element { tag, .. } => Some(tag),
            Node::Text(_) => None,
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        match self {
            Node::Element { attrs, .. } => attrs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str()),
            Node::Text(_) => None,
        }
    }

    pub fn children(&self) -> &[Node] {
        match self {
            Node::Element { children, .. } => children,
            Node::Text(_) => &[],
        }
    }

    /// Depth-first search for the first element with this tag.
    pub fn find(&self, want: &str) -> Option<&Node> {
        if self.tag() == Some(want) {
            return Some(self);
        }
        self.children().iter().find_map(|c| c.find(want))
    }

    /// Depth-first collection of every element with this tag.
    pub fn find_all<'a>(&'a self, want: &str, out: &mut Vec<&'a Node>) {
        if self.tag() == Some(want) {
            out.push(self);
        }
        for c in self.children() {
            c.find_all(want, out);
        }
    }

    /// Concatenated text content of this subtree.
    pub fn text(&self, out: &mut String) {
        match self {
            Node::Text(t) => out.push_str(t),
            Node::Element { children, .. } => {
                for c in children {
                    c.text(out);
                }
            }
        }
    }
}

const VOID: &[&str] = &["br", "img", "link", "meta", "hr", "input"];
/// Tags whose opening implicitly closes an open <p>.
const CLOSES_P: &[&str] = &[
    "p", "div", "ul", "ol", "li", "pre", "h1", "h2", "h3", "h4", "h5", "h6",
];

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(end) = s[i..].find(';').filter(|&e| e <= 8).map(|e| i + e) {
                let ent = &s[i + 1..end];
                let rep = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => ent
                        .strip_prefix('#')
                        .and_then(|n| n.parse::<u32>().ok())
                        .and_then(char::from_u32),
                };
                if let Some(c) = rep {
                    out.push(c);
                    i = end + 1;
                    continue;
                }
            }
        }
        // not an entity: copy the full UTF-8 sequence starting here
        let ch_len = s[i..].chars().next().map_or(1, |c| c.len_utf8());
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Parse a document into a synthetic "#root" element.
pub fn parse(src: &str) -> Node {
    let mut stack: Vec<Node> = vec![Node::Element {
        tag: String::from("#root"),
        attrs: Vec::new(),
        children: Vec::new(),
    }];

    let push_child = |stack: &mut Vec<Node>, node: Node| {
        if let Node::Element { children, .. } = stack.last_mut().unwrap() {
            children.push(node);
        }
    };
    // Pop the innermost open element with this tag (if any), attaching
    // every popped node to its parent on the way down.
    let close = |stack: &mut Vec<Node>, name: &str| {
        let Some(at) = stack
            .iter()
            .rposition(|n| n.tag() == Some(name))
            .filter(|&at| at > 0)
        else {
            return;
        };
        while stack.len() > at {
            let done = stack.pop().unwrap();
            if let Node::Element { children, .. } = stack.last_mut().unwrap() {
                children.push(done);
            }
        }
    };

    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let end = src[i..].find('<').map_or(b.len(), |e| i + e);
            let text = &src[i..end];
            if !text.is_empty() {
                push_child(&mut stack, Node::Text(decode_entities(text)));
            }
            i = end;
            continue;
        }
        if src[i..].starts_with("<!--") {
            i = src[i..].find("-->").map_or(b.len(), |e| i + e + 3);
            continue;
        }
        if matches!(b.get(i + 1), Some(b'!') | Some(b'?')) {
            i = src[i..].find('>').map_or(b.len(), |e| i + e + 1);
            continue;
        }
        let Some(end) = src[i..].find('>').map(|e| i + e) else {
            break; // truncated tag: drop the tail
        };
        let inner = &src[i + 1..end];
        i = end + 1;
        if let Some(name) = inner.strip_prefix('/') {
            close(&mut stack, name.trim().to_ascii_lowercase().as_str());
            continue;
        }
        let (tag, attrs, self_closed) = parse_tag(inner);
        if tag.is_empty() {
            continue;
        }
        if tag == "li" {
            close_if_open(&mut stack, "li", &close);
        }
        if CLOSES_P.contains(&tag.as_str()) {
            close_if_open(&mut stack, "p", &close);
        }
        let node = Node::Element { tag: tag.clone(), attrs, children: Vec::new() };
        if self_closed || VOID.contains(&tag.as_str()) {
            push_child(&mut stack, node);
        } else {
            stack.push(node);
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        if let Node::Element { children, .. } = stack.last_mut().unwrap() {
            children.push(done);
        }
    }
    stack.pop().unwrap()
}

/// Close `name` only if it is the innermost open element.
fn close_if_open(stack: &mut Vec<Node>, name: &str, close: &impl Fn(&mut Vec<Node>, &str)) {
    if stack.last().and_then(|n| n.tag()) == Some(name) {
        close(stack, name);
    }
}

/// "img src=\"logo.png\" /" -> (tag, attrs, self_closed)
fn parse_tag(inner: &str) -> (String, Vec<(String, String)>, bool) {
    let inner = inner.trim();
    let (inner, self_closed) = match inner.strip_suffix('/') {
        Some(rest) => (rest.trim_end(), true),
        None => (inner, false),
    };
    let name_end = inner
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(inner.len());
    let tag = inner[..name_end].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let b = inner.as_bytes();
    let mut i = name_end;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        let name_start = i;
        while i < b.len() && b[i] != b'=' && !(b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i == name_start {
            break;
        }
        let name = inner[name_start..i].to_ascii_lowercase();
        let mut value = String::new();
        while i < b.len() && (b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && (b[i] as char).is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let quote = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != quote {
                    i += 1;
                }
                value = decode_entities(&inner[vs..i]);
                i += 1; // past the closing quote
            } else {
                let vs = i;
                while i < b.len() && !(b[i] as char).is_ascii_whitespace() {
                    i += 1;
                }
                value = decode_entities(&inner[vs..i]);
            }
        }
        attrs.push((name, value));
    }
    (tag, attrs, self_closed)
}
