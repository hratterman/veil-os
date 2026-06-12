//! A small from-scratch JavaScript engine (lexer → parser → tree-walking
//! interpreter) with a DOM binding layer, enough to run the imperative
//! DOM-manipulation scripts real pages ship (set innerHTML/textContent, toggle
//! classList, create/append elements, template literals, array methods,
//! localStorage/matchMedia stubs). The browser parses HTML into a tree, lowers
//! it into a mutable arena, runs the page's scripts against it, then raises the
//! mutated arena back into a tree for the existing layout/paint pipeline.

mod ast;
mod dom;
mod interp;
mod lexer;
mod mathf;
mod parser;
mod value;

use crate::html::Node;
use alloc::string::String;
use alloc::vec::Vec;

/// Result of running a page's scripts.
pub struct JsResult {
    pub tree: Node,
    pub errors: Vec<String>,
}

/// Run `scripts` (in document order) against the DOM of `tree`, returning the
/// mutated tree. Each script's source is executed in the same global context,
/// so later scripts see earlier definitions (shared.js → content.js → inline).
pub fn run(tree: &Node, scripts: &[String]) -> JsResult {
    let dom = dom::Dom::from_tree(tree);
    let mut it = interp::Interp::new(dom);
    for src in scripts {
        it.run(src);
    }
    it.drain_deferred();
    JsResult { tree: it.dom.to_tree(), errors: it.errors }
}

/// Boot self-test entry: run the three real henryratterman.com scripts against
/// a minimal DOM skeleton and report what got populated.
pub fn selftest() {
    let skeleton = "<html><head><title id=page-title></title>\
        <meta id=page-description content=''></head><body>\
        <span id=hero-eyebrow></span><h1 id=hero-name></h1><p id=hero-tagline></p>\
        <span id=hero-status></span><blockquote id=about-pull-quote></blockquote>\
        <img id=about-headshot src=''><div id=experience-container></div>\
        <div id=projects-container></div></body></html>";
    let tree = crate::html::parse(skeleton);
    let scripts = [
        String::from(include_str!("../../assets/js/shared.js")),
        String::from(include_str!("../../assets/js/content.js")),
        String::from(include_str!("../../assets/js/render.js")),
    ];
    let res = run(&tree, &scripts);
    // Extract a couple of fields to prove the engine ran the render code.
    let name = node_text_by_id(&res.tree, "hero-name");
    let tagline = node_text_by_id(&res.tree, "hero-tagline");
    let headshot = img_src_by_id(&res.tree, "about-headshot");
    let projects = count_class(&res.tree, "project-card");
    crate::kprintln!("JS: hero-name={:?} headshot={:?} project-cards={}", name, headshot, projects);
    if !res.errors.is_empty() {
        crate::kprintln!("JS: {} script issue(s); first: {}", res.errors.len(), res.errors[0]);
    }
    if name.contains("Henry") && tagline.contains("ship") && headshot.contains("headshot") && projects >= 4 {
        crate::kprintln!("JS_OK: ES interpreter ran render() — hero, headshot, {projects} project cards injected");
    } else {
        crate::kprintln!("JS_FAIL: render did not populate the DOM (name={name:?}, cards={projects})");
    }
}

fn find_by_id<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    if n.attr("id") == Some(id) {
        return Some(n);
    }
    n.children().iter().find_map(|c| find_by_id(c, id))
}

fn node_text_by_id(tree: &Node, id: &str) -> String {
    let mut s = String::new();
    if let Some(n) = find_by_id(tree, id) {
        n.text(&mut s);
    }
    s
}

fn img_src_by_id(tree: &Node, id: &str) -> String {
    find_by_id(tree, id).and_then(|n| n.attr("src")).unwrap_or("").into()
}

fn count_class(tree: &Node, cls: &str) -> usize {
    let mut n = 0;
    count_class_rec(tree, cls, &mut n);
    n
}

fn count_class_rec(node: &Node, cls: &str, n: &mut usize) {
    if node.attr("class").map(|c| c.split_whitespace().any(|w| w == cls)).unwrap_or(false) {
        *n += 1;
    }
    for c in node.children() {
        count_class_rec(c, cls, n);
    }
}
