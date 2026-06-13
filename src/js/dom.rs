//! A flat, index-addressable DOM arena for the JS engine to mutate. The
//! browser parses HTML into an `html::Node` tree; we lower it into this arena
//! (so JS can hold stable element handles), let scripts mutate it, then raise
//! it back into an `html::Node` tree for the existing layout/paint pipeline.

use crate::html::{self, Node};
use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;

pub struct DomNode {
    pub tag: String,            // element tag, or "#text"
    pub attrs: Vec<(String, String)>,
    pub text: String,           // for "#text" nodes
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

pub struct Dom {
    pub nodes: Vec<DomNode>,
    pub root: usize, // the #root element
}

impl DomNode {
    fn elem(tag: &str) -> DomNode {
        DomNode { tag: String::from(tag), attrs: Vec::new(), text: String::new(), parent: None, children: Vec::new() }
    }
    fn text(t: &str) -> DomNode {
        DomNode { tag: String::from("#text"), attrs: Vec::new(), text: String::from(t), parent: None, children: Vec::new() }
    }
    pub fn is_text(&self) -> bool {
        self.tag == "#text"
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
    pub fn set_attr(&mut self, name: &str, val: &str) {
        if let Some(a) = self.attrs.iter_mut().find(|(n, _)| n == name) {
            a.1 = String::from(val);
        } else {
            self.attrs.push((String::from(name), String::from(val)));
        }
    }
}

impl Dom {
    pub fn from_tree(tree: &Node) -> Dom {
        let mut dom = Dom { nodes: Vec::new(), root: 0 };
        let root = dom.lower(tree, None);
        dom.root = root;
        dom
    }

    fn lower(&mut self, n: &Node, parent: Option<usize>) -> usize {
        let idx = self.nodes.len();
        match n {
            Node::Text(t) => {
                let mut nd = DomNode::text(t);
                nd.parent = parent;
                self.nodes.push(nd);
                idx
            }
            Node::Element { tag, attrs, children } => {
                let mut nd = DomNode::elem(tag);
                nd.attrs = attrs.clone();
                nd.parent = parent;
                self.nodes.push(nd);
                let mut kids = Vec::with_capacity(children.len());
                for c in children {
                    kids.push(self.lower(c, Some(idx)));
                }
                self.nodes[idx].children = kids;
                idx
            }
        }
    }

    /// Raise the arena back into an owned html::Node tree.
    pub fn to_tree(&self) -> Node {
        self.raise(self.root)
    }

    fn raise(&self, idx: usize) -> Node {
        let nd = &self.nodes[idx];
        if nd.is_text() {
            Node::Text(nd.text.clone())
        } else {
            let children = nd.children.iter().map(|&c| self.raise(c)).collect();
            Node::Element { tag: nd.tag.clone(), attrs: nd.attrs.clone(), children }
        }
    }

    // --- node creation / structure -----------------------------------------

    pub fn create_element(&mut self, tag: &str) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(DomNode::elem(&tag.to_ascii_lowercase()));
        idx
    }

    pub fn create_text(&mut self, t: &str) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(DomNode::text(t));
        idx
    }

    pub fn create_fragment(&mut self) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(DomNode::elem("#fragment"));
        idx
    }

    pub fn append_child(&mut self, parent: usize, child: usize) {
        // A document fragment dissolves: its children are moved, not the node.
        if self.nodes[child].tag == "#fragment" {
            let kids = core::mem::take(&mut self.nodes[child].children);
            for c in kids {
                self.nodes[c].parent = None;
                self.append_child(parent, c);
            }
            return;
        }
        // detach from old parent
        if let Some(op) = self.nodes[child].parent {
            self.nodes[op].children.retain(|&c| c != child);
        }
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    /// Insert `child` before `reference` in `parent`'s child list. If
    /// `reference` is None or not a child, append at the end.
    pub fn insert_before(&mut self, parent: usize, child: usize, reference: Option<usize>) {
        if self.nodes[child].tag == "#fragment" {
            let kids = core::mem::take(&mut self.nodes[child].children);
            for c in kids {
                self.nodes[c].parent = None;
                self.insert_before(parent, c, reference);
            }
            return;
        }
        if let Some(op) = self.nodes[child].parent {
            self.nodes[op].children.retain(|&c| c != child);
        }
        self.nodes[child].parent = Some(parent);
        let pos = reference
            .and_then(|r| self.nodes[parent].children.iter().position(|&c| c == r));
        match pos {
            Some(p) => self.nodes[parent].children.insert(p, child),
            None => self.nodes[parent].children.push(child),
        }
    }

    pub fn remove_child(&mut self, parent: usize, child: usize) {
        self.nodes[parent].children.retain(|&x| x != child);
        if self.nodes[child].parent == Some(parent) {
            self.nodes[child].parent = None;
        }
    }

    /// DOM nodeType: 1 element, 3 text, 11 document-fragment.
    pub fn node_type(&self, idx: usize) -> u32 {
        match self.nodes[idx].tag.as_str() {
            "#text" => 3,
            "#fragment" => 11,
            "#comment" => 8,
            _ => 1,
        }
    }

    // --- lookups ------------------------------------------------------------

    pub fn get_by_id(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.attr("id") == Some(id))
    }

    pub fn get_by_tag<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = usize> + 'a {
        let t = tag.to_ascii_lowercase();
        self.nodes.iter().enumerate().filter(move |(_, n)| n.tag == t).map(|(i, _)| i)
    }

    /// Minimal querySelector: #id, .class, tag, tag.class, tag[attr="v"].
    pub fn query(&self, sel: &str) -> Option<usize> {
        self.query_all(sel).into_iter().next()
    }

    pub fn query_all(&self, sel: &str) -> Vec<usize> {
        let sel = sel.trim();
        let mut out = Vec::new();
        // Comma list: union of each selector's matches.
        if sel.contains(',') {
            for part in sel.split(',') {
                for m in self.query_all(part.trim()) {
                    if !out.contains(&m) {
                        out.push(m);
                    }
                }
            }
            return out;
        }
        // Descendant combinator "A B C": a node matches the rightmost compound
        // and has ancestors matching the earlier compounds in order.
        let parts: Vec<&str> = sel.split_whitespace().collect();
        if parts.len() > 1 {
            let last = parts[parts.len() - 1];
            for (i, n) in self.nodes.iter().enumerate() {
                if n.is_text() || !self.matches(i, n, last) {
                    continue;
                }
                if self.ancestors_match(i, &parts[..parts.len() - 1]) {
                    out.push(i);
                }
            }
            return out;
        }
        for (i, n) in self.nodes.iter().enumerate() {
            if n.is_text() {
                continue;
            }
            if self.matches(i, n, sel) {
                out.push(i);
            }
        }
        out
    }

    /// Walking up from `idx`, do the ancestor compounds match in order (each
    /// somewhere above, right-to-left)?
    fn ancestors_match(&self, idx: usize, compounds: &[&str]) -> bool {
        let mut remaining = compounds.len();
        let mut cur = self.nodes[idx].parent;
        while let Some(p) = cur {
            if remaining > 0 && self.matches(p, &self.nodes[p], compounds[remaining - 1]) {
                remaining -= 1;
                if remaining == 0 {
                    return true;
                }
            }
            cur = self.nodes[p].parent;
        }
        remaining == 0
    }

    fn matches(&self, _idx: usize, n: &DomNode, sel: &str) -> bool {
        if let Some(id) = sel.strip_prefix('#') {
            return n.attr("id") == Some(id);
        }
        if let Some(cls) = sel.strip_prefix('.') {
            return self.has_class(n, cls);
        }
        // tag[attr="v"]
        if let Some(b) = sel.find('[') {
            let tag = &sel[..b];
            if !tag.is_empty() && n.tag != tag.to_ascii_lowercase() {
                return false;
            }
            let inner = &sel[b + 1..sel.find(']').unwrap_or(sel.len())];
            if let Some(eq) = inner.find('=') {
                let an = inner[..eq].trim();
                let av = inner[eq + 1..].trim().trim_matches(|c| c == '"' || c == '\'');
                return n.attr(an) == Some(av);
            }
            return n.attr(inner.trim()).is_some();
        }
        // tag.class
        if let Some(dot) = sel.find('.') {
            let tag = &sel[..dot];
            let cls = &sel[dot + 1..];
            return n.tag == tag.to_ascii_lowercase() && self.has_class(n, cls);
        }
        n.tag == sel.to_ascii_lowercase()
    }

    pub fn has_class(&self, n: &DomNode, cls: &str) -> bool {
        n.attr("class").map(|c| c.split_whitespace().any(|w| w == cls)).unwrap_or(false)
    }

    // --- content getters/setters --------------------------------------------

    pub fn text_content(&self, idx: usize) -> String {
        let mut s = String::new();
        self.collect_text(idx, &mut s);
        s
    }

    fn collect_text(&self, idx: usize, out: &mut String) {
        let n = &self.nodes[idx];
        if n.is_text() {
            out.push_str(&n.text);
        } else {
            for &c in &n.children {
                self.collect_text(c, out);
            }
        }
    }

    pub fn set_text_content(&mut self, idx: usize, t: &str) {
        // Detach the existing children (the arena keeps the orphans, but they're
        // no longer referenced from this subtree).
        for c in core::mem::take(&mut self.nodes[idx].children) {
            self.nodes[c].parent = None;
        }
        // `textContent = ""` clears the element to *no* children — it must NOT
        // leave an empty text node behind. React's `resetTextContent` sets `""`
        // before appending the real child; a stray empty text node corrupts the
        // child list and the reconciler's sibling walk.
        if t.is_empty() {
            return;
        }
        let tn = self.create_text(t);
        self.nodes[tn].parent = Some(idx);
        self.nodes[idx].children = vec![tn];
    }

    pub fn set_inner_html(&mut self, idx: usize, html: &str) {
        let tree = html::parse(html); // a #root wrapper
        // lower the parsed tree's children into this arena under `idx`
        let mut new_kids = Vec::new();
        if let Node::Element { children, .. } = &tree {
            for c in children {
                let cidx = self.lower(c, Some(idx));
                new_kids.push(cidx);
            }
        }
        self.nodes[idx].children = new_kids;
    }

    pub fn inner_html(&self, idx: usize) -> String {
        let mut s = String::new();
        for &c in &self.nodes[idx].children {
            self.serialize(c, &mut s);
        }
        s
    }

    fn serialize(&self, idx: usize, out: &mut String) {
        let n = &self.nodes[idx];
        if n.is_text() {
            out.push_str(&n.text);
            return;
        }
        out.push('<');
        out.push_str(&n.tag);
        for (k, v) in &n.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(v);
            out.push('"');
        }
        out.push('>');
        for &c in &n.children {
            self.serialize(c, out);
        }
        out.push_str("</");
        out.push_str(&n.tag);
        out.push('>');
    }

    // --- classList ----------------------------------------------------------

    pub fn class_add(&mut self, idx: usize, cls: &str) {
        let cur = self.nodes[idx].attr("class").unwrap_or("").to_string();
        if !cur.split_whitespace().any(|w| w == cls) {
            let new = if cur.is_empty() { cls.to_string() } else { format_class(&cur, cls) };
            self.nodes[idx].set_attr("class", &new);
        }
    }

    pub fn class_remove(&mut self, idx: usize, cls: &str) {
        let cur = self.nodes[idx].attr("class").unwrap_or("").to_string();
        let new: Vec<&str> = cur.split_whitespace().filter(|&w| w != cls).collect();
        self.nodes[idx].set_attr("class", &new.join(" "));
    }

    pub fn class_contains(&self, idx: usize, cls: &str) -> bool {
        self.nodes[idx].attr("class").map(|c| c.split_whitespace().any(|w| w == cls)).unwrap_or(false)
    }

    pub fn class_toggle(&mut self, idx: usize, cls: &str) -> bool {
        if self.class_contains(idx, cls) {
            self.class_remove(idx, cls);
            false
        } else {
            self.class_add(idx, cls);
            true
        }
    }

    // --- inline style -------------------------------------------------------

    pub fn set_style(&mut self, idx: usize, prop: &str, val: &str) {
        let cur = self.nodes[idx].attr("style").unwrap_or("").to_string();
        let mut parts: Vec<(String, String)> = Vec::new();
        for decl in cur.split(';') {
            if let Some(c) = decl.find(':') {
                parts.push((decl[..c].trim().to_string(), decl[c + 1..].trim().to_string()));
            }
        }
        let prop_kebab = camel_to_kebab(prop);
        if let Some(p) = parts.iter_mut().find(|(k, _)| *k == prop_kebab) {
            p.1 = val.to_string();
        } else {
            parts.push((prop_kebab, val.to_string()));
        }
        let s: Vec<String> = parts.iter().map(|(k, v)| alloc::format!("{k}: {v}")).collect();
        self.nodes[idx].set_attr("style", &s.join("; "));
    }

    pub fn get_style(&self, idx: usize, prop: &str) -> String {
        let cur = self.nodes[idx].attr("style").unwrap_or("");
        let prop_kebab = camel_to_kebab(prop);
        for decl in cur.split(';') {
            if let Some(c) = decl.find(':') {
                if decl[..c].trim() == prop_kebab {
                    return decl[c + 1..].trim().to_string();
                }
            }
        }
        String::new()
    }
}

fn format_class(cur: &str, add: &str) -> String {
    let mut s = String::from(cur);
    s.push(' ');
    s.push_str(add);
    s
}

fn camel_to_kebab(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

use alloc::string::ToString;
