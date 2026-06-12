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
pub mod jit;
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

/// ES6+ feature self-test: exercise classes, destructuring, default params,
/// spread, template literals, arrow fns, Map/Set, Object/Array statics, optional
/// chaining, nullish coalescing, generators-ish, and async/await with a resolved
/// Promise — writing the combined result into a DOM node we read back.
pub fn es6_selftest() {
    let skeleton = "<html><body><div id=out></div></body></html>";
    let tree = crate::html::parse(skeleton);
    let src = r#"
        class Animal {
          constructor(name) { this.name = name; this.legs = 4; }
          describe() { return `${this.name} has ${this.legs} legs`; }
          static make(n) { return new Animal(n); }
        }
        class Dog extends Animal {
          constructor(name) { super(name); this.sound = "woof"; }
          describe() { return super.describe() + ` and says ${this.sound}`; }
        }
        const d = new Dog("Rex");
        const { name, sound } = d;
        const nums = [1, 2, 3, 4, 5];
        const [first, ...rest] = nums;
        const sum = nums.reduce((a, b) => a + b, 0);
        const doubled = nums.map(n => n * 2).filter(n => n > 4);
        const greet = (who = "world") => `hi ${who}`;
        const m = new Map();
        m.set("a", 1); m.set("b", 2);
        const s = new Set([1, 1, 2, 3, 3]);
        const obj = { x: 1, y: 2 };
        const merged = { ...obj, z: 3 };
        const keys = Object.keys(merged).join(",");
        const maybe = null;
        const safe = maybe?.foo ?? "fallback";
        async function compute() {
          const base = await Promise.resolve(10);
          return base + sum;
        }
        let total = 0;
        (async () => { total = await compute(); })();
        const parsed = JSON.parse('{"ok":true,"n":42}');
        const out = [
          d.describe(),
          `name=${name} sound=${sound}`,
          `first=${first} rest=${rest.join("-")}`,
          `sum=${sum} doubled=${doubled.join(",")}`,
          greet(),
          `map.size=${m.size} map.a=${m.get("a")}`,
          `set.size=${s.size}`,
          `keys=${keys}`,
          `safe=${safe}`,
          `total=${total}`,
          `parsed.n=${parsed.n} parsed.ok=${parsed.ok}`,
          `instanceof=${d instanceof Animal}`
        ].join(" | ");
        document.getElementById("out").textContent = out;
    "#;
    let res = run(&tree, &[String::from(src)]);
    let out = node_text_by_id(&res.tree, "out");
    crate::kprintln!("JS_ES6: {out}");
    if !res.errors.is_empty() {
        crate::kprintln!("JS_ES6: {} issue(s); first: {}", res.errors.len(), res.errors[0]);
    }
    // Acceptance: every feature produced its expected substring.
    let checks = [
        "Rex has 4 legs and says woof",
        "name=Rex sound=woof",
        "first=1 rest=2-3-4-5",
        "sum=15 doubled=6,8,10",
        "hi world",
        "map.size=2 map.a=1",
        "set.size=3",
        "keys=x,y,z",
        "safe=fallback",
        "total=25",
        "parsed.n=42 parsed.ok=true",
        "instanceof=true",
    ];
    let pass = checks.iter().all(|c| out.contains(c));
    if pass {
        crate::kprintln!("JS_ES6_OK: classes, destructuring, defaults, spread, Map/Set, Object.keys, ?./??, async/await, Promise, JSON, instanceof all work");
    } else {
        let missing: alloc::vec::Vec<&str> = checks.iter().copied().filter(|c| !out.contains(c)).collect();
        crate::kprintln!("JS_ES6_FAIL: missing {:?}", missing);
    }
}

/// JIT self-test: compile a numeric hot-loop function to native AArch64 and
/// confirm it (a) returns the same result as the interpreter and (b) is much
/// faster, timed on the cycle counter. This is the from-scratch JS JIT.
pub fn jit_selftest() {
    use value::Val;
    let src = r#"
        function bench(n) {
          let acc = 0;
          for (let i = 0; i < n; i++) {
            let x = i % 7;
            acc = acc + x * x - (i % 3) + (i / 2 - x);
            if (acc > 1000000) { acc = acc - 1000000; }
          }
          return acc;
        }
    "#;
    let tree = crate::html::parse("<html><body></body></html>");
    let dom = dom::Dom::from_tree(&tree);
    let mut it = interp::Interp::new(dom);
    it.run(src);
    let func = match it.global_val("bench") {
        Some(v @ Val::Func(..)) => v,
        _ => {
            crate::kprintln!("JS_JIT_FAIL: bench not defined");
            return;
        }
    };
    let rc = match &func {
        Val::Func(rc, _) => rc.clone(),
        _ => return,
    };

    let cyc = || -> u64 {
        let v: u64;
        unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) v) };
        v
    };
    // Keep N modest so the interpreted baseline doesn't slow the debug boot;
    // the speedup ratio is independent of N (both scale linearly).
    let n = 40_000.0f64;

    // Interpreted baseline (JIT disabled).
    it.set_jit(false);
    let t0 = cyc();
    let interp_res = it.call(func.clone(), Val::Undef, alloc::vec![Val::Num(n)]);
    let t1 = cyc();
    let interp_val = interp_res.map(|v| v.as_num()).unwrap_or(f64::NAN);

    // JIT: compile directly and run native.
    let Some(code) = jit::compile(&rc) else {
        crate::kprintln!("JS_JIT_FAIL: bench did not compile (should fit the numeric subset)");
        return;
    };
    let t2 = cyc();
    let jit_val = code.run(&[n]);
    let t3 = cyc();

    let (ic, jc) = (t1.wrapping_sub(t0), t3.wrapping_sub(t2));
    let speed = if jc > 0 { ic / jc } else { 0 };
    let agree = (interp_val - jit_val).abs() < 1e-6;
    crate::kprintln!(
        "JS_JIT: bench(40000) interp={interp_val} ({ic} cyc), jit={jit_val} ({jc} cyc), ~{speed}x (agree={agree})"
    );
    if agree && speed >= 50 {
        crate::kprintln!("JS_JIT_FAST: native AArch64 JS JIT is {speed}x faster than the interpreter (>=50x)");
    } else if agree {
        crate::kprintln!("JS_JIT_OK: native codegen agrees ({speed}x; wanted >=50x)");
    } else {
        crate::kprintln!("JS_JIT_FAIL: results disagree (interp={interp_val} jit={jit_val})");
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
