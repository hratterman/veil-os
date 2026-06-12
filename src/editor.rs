//! M41 step 14: a real code editor (App::Editor).
//!
//! A cursor-based text buffer (insert/delete anywhere, arrow navigation,
//! Home/End/PageUp/Down), multi-level **undo/redo**, **find** (Ctrl+F) and
//! **find/replace** (Ctrl+H, replace-all), **go-to-line** (Ctrl+G), **auto-indent**
//! + **bracket auto-close**, line numbers, a **file-tree sidebar** (Ctrl+B), and
//! syntax highlighting for Rust / C / JS / Python / HTML / CSS / JSON / Shell /
//! Markdown. Replaces the M18 append-only editor.

use crate::fb::Framebuffer;
use crate::freetype::FontId;
use crate::wm::{App, Window, TOOLBAR_H};
use crate::{fs, kprintln};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const ED_BG: u32 = 0xff1e_1e1e;
const ED_GUTTER: u32 = 0xff25_2526;
const ED_LINENO: u32 = 0xff85_8585;
const ED_FG: u32 = 0xffd4_d4d4;
const ED_KW: u32 = 0xff56_9cd6;
const ED_STR: u32 = 0xffce_9178;
const ED_NUM: u32 = 0xffb5_cea8;
const ED_COMMENT: u32 = 0xff6a_9955;
const ED_SEL: u32 = 0xff264f_78;
const ED_TREE_BG: u32 = 0xff20_2024;
const ED_MATCH: u32 = 0xff60_5020;

const LH: usize = 17;
const GUT_W: usize = 46;
const TREE_W: usize = 130;
const FONT_PX: u16 = 14;
const MAX: usize = 64 * 1024;

const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_UP: u16 = 103;
const KEY_DOWN: u16 = 108;
const KEY_HOME: u16 = 102;
const KEY_END: u16 = 107;
const KEY_PGUP: u16 = 104;
const KEY_PGDN: u16 = 109;
const KEY_DELETE: u16 = 111;
const KEY_ENTER: u16 = 28;
const KEY_TAB: u16 = 15;
const KEY_BACKSPACE: u16 = 14;
const KEY_ESC: u16 = 1;

#[derive(PartialEq, Clone, Copy)]
enum Bar {
    None,
    Find,
    Replace,
    Goto,
}

pub struct EditorState {
    file: String,
    text: String,
    cursor: usize, // byte offset of the insertion point
    scroll: usize, // top visible row
    status: String,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    typing: bool, // coalesce runs of typed chars into one undo
    bar: Bar,
    find_q: String,
    replace_q: String,
    bar_field: u8, // for replace: 0=find, 1=replace
    tree: bool,
    files: Vec<String>,
}

impl EditorState {
    pub fn open(file: &str) -> EditorState {
        let (text, status) = match fs::read_file(file) {
            Some(data) => {
                kprintln!("EDITOR: opened {file} ({} bytes)", data.len());
                (decode(&data), format!("{} bytes", data.len()))
            }
            None => {
                let s = match fs::write_file(file, b"") {
                    Ok(()) => "new file",
                    Err(()) => "create failed",
                };
                kprintln!("EDITOR: {file} missing -> {s}");
                (String::new(), String::from(s))
            }
        };
        EditorState {
            file: String::from(file),
            text,
            cursor: 0,
            scroll: 0,
            status,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
            bar: Bar::None,
            find_q: String::new(),
            replace_q: String::new(),
            bar_field: 0,
            tree: false,
            files: Vec::new(),
        }
    }

    // --- buffer edits -------------------------------------------------------

    fn snapshot(&mut self) {
        self.undo.push((self.text.clone(), self.cursor));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn insert(&mut self, s: &str) {
        if self.text.len() + s.len() > MAX {
            return;
        }
        self.cursor = self.cursor.min(self.text.len());
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.status = String::from("edited");
    }

    fn type_char(&mut self, ch: char) {
        if !self.typing {
            self.snapshot();
            self.typing = true;
        }
        // Bracket / quote auto-close.
        let close = match ch {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            _ => None,
        };
        let mut buf = [0u8; 4];
        self.insert(ch.encode_utf8(&mut buf));
        if let Some(c) = close {
            let at = self.cursor;
            self.text.insert(at, c); // closing char, cursor stays before it
        }
    }

    fn newline(&mut self) {
        self.snapshot();
        self.typing = false;
        // auto-indent: copy leading whitespace of the current line.
        let ls = self.line_start(self.cursor);
        let indent: String = self.text[ls..self.cursor]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let extra = if self.text[..self.cursor].ends_with(['{', '(', '[', ':']) { "    " } else { "" };
        let ins = format!("\n{indent}{extra}");
        self.insert(&ins);
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.snapshot();
        self.typing = false;
        let prev = self.text[..self.cursor].chars().next_back().map(|c| c.len_utf8()).unwrap_or(1);
        self.text.replace_range(self.cursor - prev..self.cursor, "");
        self.cursor -= prev;
        self.status = String::from("edited");
    }

    fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        self.snapshot();
        self.typing = false;
        let n = self.text[self.cursor..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        self.text.replace_range(self.cursor..self.cursor + n, "");
        self.status = String::from("edited");
    }

    pub fn paste(&mut self, s: &str) {
        self.snapshot();
        self.typing = false;
        let clean: String = s.chars().filter(|&c| c == '\n' || c == '\t' || (' '..='~').contains(&c)).collect();
        self.insert(&clean);
    }

    fn undo_op(&mut self) {
        if let Some((t, c)) = self.undo.pop() {
            self.redo.push((self.text.clone(), self.cursor));
            self.text = t;
            self.cursor = c.min(self.text.len());
            self.typing = false;
            self.status = String::from("undo");
        }
    }

    fn redo_op(&mut self) {
        if let Some((t, c)) = self.redo.pop() {
            self.undo.push((self.text.clone(), self.cursor));
            self.text = t;
            self.cursor = c.min(self.text.len());
            self.typing = false;
            self.status = String::from("redo");
        }
    }

    // --- cursor navigation --------------------------------------------------

    fn line_start(&self, at: usize) -> usize {
        self.text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }
    fn line_end(&self, at: usize) -> usize {
        self.text[at..].find('\n').map(|i| at + i).unwrap_or(self.text.len())
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= self.text[..self.cursor].chars().next_back().map(|c| c.len_utf8()).unwrap_or(1);
        }
        self.typing = false;
    }
    fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor += self.text[self.cursor..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
        self.typing = false;
    }
    fn col_in_line(&self) -> usize {
        self.text[self.line_start(self.cursor)..self.cursor].chars().count()
    }
    fn move_vert(&mut self, dir: isize) {
        let col = self.col_in_line();
        if dir < 0 {
            let ls = self.line_start(self.cursor);
            if ls == 0 {
                return;
            }
            let prev_start = self.line_start(ls - 1);
            self.cursor = nth_col(&self.text, prev_start, ls - 1, col);
        } else {
            let le = self.line_end(self.cursor);
            if le >= self.text.len() {
                return;
            }
            let next_start = le + 1;
            let next_end = self.line_end(next_start);
            self.cursor = nth_col(&self.text, next_start, next_end, col);
        }
        self.typing = false;
    }
    fn home(&mut self) {
        self.cursor = self.line_start(self.cursor);
        self.typing = false;
    }
    fn end(&mut self) {
        self.cursor = self.line_end(self.cursor);
        self.typing = false;
    }

    fn goto_line(&mut self, n: usize) {
        let mut start = 0;
        for _ in 1..n {
            match self.text[start..].find('\n') {
                Some(i) => start += i + 1,
                None => break,
            }
        }
        self.cursor = start;
    }

    fn find_next(&mut self, from: usize) {
        if self.find_q.is_empty() {
            return;
        }
        if let Some(i) = self.text[from..].find(&self.find_q).map(|i| from + i) {
            self.cursor = i + self.find_q.len();
        } else if let Some(i) = self.text.find(&self.find_q) {
            self.cursor = i + self.find_q.len(); // wrap
        }
    }

    fn replace_all(&mut self) {
        if self.find_q.is_empty() {
            return;
        }
        self.snapshot();
        let n = self.text.matches(&self.find_q).count();
        self.text = self.text.replace(&self.find_q, &self.replace_q);
        self.cursor = self.cursor.min(self.text.len());
        self.status = format!("replaced {n}");
        kprintln!("EDITOR: replaced {n} occurrence(s) of {:?}", self.find_q);
    }

    fn save(&mut self) {
        match fs::write_file(&self.file, self.text.as_bytes()) {
            Ok(()) => {
                self.status = format!("saved {} bytes", self.text.len());
                kprintln!("EDITOR: saved {} bytes to {}", self.text.len(), self.file);
                kprintln!("EDITOR_OK");
            }
            Err(()) => self.status = String::from("save FAILED"),
        }
    }

    fn cursor_line(&self) -> usize {
        self.text[..self.cursor].matches('\n').count()
    }

    fn ensure_visible(&mut self, rows: usize) {
        let line = self.cursor_line();
        if line < self.scroll {
            self.scroll = line;
        } else if rows > 0 && line >= self.scroll + rows {
            self.scroll = line + 1 - rows;
        }
    }
}

/// Byte offset of the `col`-th char in [start, end] (clamped to end).
fn nth_col(text: &str, start: usize, end: usize, col: usize) -> usize {
    let mut p = start;
    for _ in 0..col {
        if p >= end {
            break;
        }
        p += text[p..end].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    p.min(end)
}

fn decode(data: &[u8]) -> String {
    data.iter().map(|&b| b as char).filter(|&c| c == '\n' || c == '\t' || (' '..='~').contains(&c)).collect()
}

fn rows_of(ch: usize) -> usize {
    (ch.saturating_sub(TOOLBAR_H as usize + 22)) / LH
}

// --- WM-facing handlers -------------------------------------------------------

/// Printable char input (also handles the open bars). Returns true if consumed.
pub fn char_input(win: &mut Window, ch: char) {
    {
        let App::Editor(st) = &mut win.app else { return };
        if st.bar != Bar::None {
            if !ch.is_control() {
                if st.bar == Bar::Replace && st.bar_field == 1 {
                    st.replace_q.push(ch);
                } else {
                    st.find_q.push(ch);
                }
            }
        } else if ch == '\u{8}' {
            st.backspace();
        } else if !ch.is_control() {
            st.type_char(ch);
        }
    }
    render(win);
}

/// Non-character keys (arrows, enter, etc.). Returns true if consumed.
pub fn key(win: &mut Window, code: u16, shift: bool) -> bool {
    let rows = rows_of(win.ch);
    let App::Editor(st) = &mut win.app else { return false };
    // Open-bar keys first.
    if st.bar != Bar::None {
        match code {
            KEY_ESC => st.bar = Bar::None,
            KEY_ENTER if st.bar == Bar::Goto => {
                if let Ok(n) = st.find_q.trim().parse::<usize>() {
                    st.goto_line(n.max(1));
                }
                st.bar = Bar::None;
                st.find_q.clear();
            }
            KEY_ENTER if st.bar == Bar::Replace => st.replace_all(),
            KEY_ENTER => st.find_next(st.cursor),
            KEY_TAB if st.bar == Bar::Replace => st.bar_field ^= 1,
            KEY_BACKSPACE => {
                if st.bar == Bar::Replace && st.bar_field == 1 {
                    st.replace_q.pop();
                } else {
                    st.find_q.pop();
                }
            }
            _ => {
                render(win);
                return false;
            }
        }
        st.ensure_visible(rows);
        render(win);
        return true;
    }
    let consumed = match code {
        KEY_LEFT => {
            st.move_left();
            true
        }
        KEY_RIGHT => {
            st.move_right();
            true
        }
        KEY_UP => {
            st.move_vert(-1);
            true
        }
        KEY_DOWN => {
            st.move_vert(1);
            true
        }
        KEY_HOME => {
            st.home();
            true
        }
        KEY_END => {
            st.end();
            true
        }
        KEY_PGUP => {
            for _ in 0..rows {
                st.move_vert(-1);
            }
            true
        }
        KEY_PGDN => {
            for _ in 0..rows {
                st.move_vert(1);
            }
            true
        }
        KEY_ENTER => {
            st.newline();
            true
        }
        KEY_TAB => {
            st.snapshot();
            st.insert("    ");
            st.typing = false;
            true
        }
        KEY_DELETE => {
            st.delete();
            true
        }
        _ => false,
    };
    let _ = shift;
    if consumed {
        st.ensure_visible(rows);
        render(win);
    }
    consumed
}

/// Ctrl shortcuts. Returns true if consumed.
pub fn ctrl_key(win: &mut Window, code: u16, shift: bool) -> bool {
    const C_S: u16 = 31;
    const C_Z: u16 = 44;
    const C_Y: u16 = 21;
    const C_F: u16 = 33;
    const C_H: u16 = 35;
    const C_G: u16 = 34;
    const C_B: u16 = 48;
    {
        let App::Editor(st) = &mut win.app else { return false };
        match code {
            C_S => st.save(),
            C_Z if shift => st.redo_op(),
            C_Z => st.undo_op(),
            C_Y => st.redo_op(),
            C_F => {
                st.bar = Bar::Find;
                st.bar_field = 0;
            }
            C_H => {
                st.bar = Bar::Replace;
                st.bar_field = 0;
            }
            C_G => {
                st.bar = Bar::Goto;
                st.find_q.clear();
            }
            C_B => {
                st.tree = !st.tree;
                if st.tree {
                    st.files = fs::list_root().unwrap_or_default().into_iter().map(|(n, _)| n).collect();
                    st.files.sort();
                }
            }
            _ => return false,
        }
    }
    render(win);
    true
}

/// A mouse click: toolbar buttons, the file tree, or set the cursor.
pub fn click(win: &mut Window, rx: isize, ry: isize) -> Option<String> {
    let (cw, ch) = (win.cw as isize, win.ch);
    // Toolbar LOD / SAV.
    if ry >= 2 && ry < 26 {
        let App::Editor(st) = &mut win.app else { return None };
        if rx >= cw - 52 && rx < cw - 8 {
            st.save();
        } else if rx >= cw - 100 && rx < cw - 56 {
            if let Some(d) = fs::read_file(&st.file) {
                st.text = decode(&d);
                st.cursor = 0;
                st.status = format!("loaded {} bytes", d.len());
                kprintln!("EDITOR: loaded {} bytes from {}", d.len(), st.file);
                kprintln!("EDITOR_OK");
            }
        }
        render(win);
        return None;
    }
    let top = TOOLBAR_H as isize;
    // File tree sidebar (click opens a file).
    let tree_on = matches!(&win.app, App::Editor(st) if st.tree);
    if tree_on && rx < TREE_W as isize {
        let App::Editor(st) = &win.app else { return None };
        let i = ((ry - top - 2) / LH as isize) as usize;
        if let Some(name) = st.files.get(i).cloned() {
            return Some(name);
        }
        return None;
    }
    // Set the cursor from (row, x). Gather the target line first (drops the st
    // borrow), measure with the framebuffer, then write the cursor back.
    let xoff = if tree_on { TREE_W } else { 0 };
    let (ls, line) = {
        let App::Editor(st) = &win.app else { return None };
        let row = ((ry - top - 2).max(0) / LH as isize) as usize + st.scroll;
        let mut ls = 0;
        for _ in 0..row {
            match st.text[ls..].find('\n') {
                Some(j) => ls = ls + j + 1,
                None => break,
            }
        }
        let le = st.text[ls..].find('\n').map(|j| ls + j).unwrap_or(st.text.len());
        (ls, st.text[ls..le].to_string())
    };
    let target = (rx - (xoff + GUT_W + 6) as isize).max(0) as i64;
    let mut p = 0usize;
    {
        let fb = win.canvas_fb();
        let mut acc = 0i64;
        let lb = line.as_bytes();
        while p < line.len() {
            let cw1 = line[p..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            let g = fb.measure_text(&line[p..p + cw1], FontId::Mono, FONT_PX).0 as i64;
            if acc + g / 2 >= target {
                break;
            }
            acc += g;
            p += cw1;
        }
        let _ = lb;
    }
    if let App::Editor(st) = &mut win.app {
        st.cursor = (ls + p).min(st.text.len());
        st.typing = false;
    }
    render(win);
    None
}

pub fn wheel(win: &mut Window, notches: i32) -> bool {
    let rows = rows_of(win.ch);
    let App::Editor(st) = &mut win.app else { return false };
    let nlines = st.text.matches('\n').count() + 1;
    let max = nlines.saturating_sub(rows);
    st.scroll = (st.scroll as isize - notches as isize * 3).clamp(0, max as isize) as usize;
    true
}

// --- rendering ----------------------------------------------------------------

pub fn render(win: &mut Window) {
    let (file, status, text, scroll, cursor, bar, find_q, replace_q, bar_field, tree, files) = {
        let App::Editor(st) = &win.app else { return };
        (
            st.file.clone(), st.status.clone(), st.text.clone(), st.scroll, st.cursor, st.bar,
            st.find_q.clone(), st.replace_q.clone(), st.bar_field, st.tree, st.files.clone(),
        )
    };
    let lang = editor_lang(&file);
    let (cw, ch) = (win.cw, win.ch);
    let rows = rows_of(ch);
    let top = TOOLBAR_H as usize;
    let xoff = if tree { TREE_W } else { 0 };

    let lines: Vec<&str> = text.split('\n').collect();
    let cur_line = text[..cursor].matches('\n').count();
    let cur_col = text[line_start_of(&text, cursor)..cursor].chars().count();

    let fb = win.canvas_fb();
    fb.fill_rect(0, top, cw, ch - top, ED_BG);
    // File tree sidebar.
    if tree {
        fb.fill_rect(0, top, TREE_W, ch - top, ED_TREE_BG);
        for (i, f) in files.iter().enumerate() {
            let y = top + 2 + i * LH;
            if y + LH > ch {
                break;
            }
            let col = if f.eq_ignore_ascii_case(&file) { ED_KW } else { ED_FG };
            fb.draw_text(6, y + 1, f, FontId::Mono, 12, col);
        }
    }
    fb.fill_rect(xoff, top, GUT_W, ch - top, ED_GUTTER);
    toolbar(&fb, cw, &file, &status);

    for r in 0..rows {
        let lineno = scroll + r;
        let Some(line) = lines.get(lineno) else { break };
        let y = top + 2 + r * LH;
        // gutter line number
        let num = format!("{}", lineno + 1);
        let (nw, _) = fb.measure_text(&num, FontId::Mono, 13);
        fb.draw_text(xoff + GUT_W - nw - 6, y + 1, &num, FontId::Mono, 13, ED_LINENO);
        // find-match highlight on this line
        if bar != Bar::Goto && !find_q.is_empty() {
            let mut from = 0;
            while let Some(mi) = line[from..].find(&find_q) {
                let mstart = from + mi;
                let px = fb.measure_text(&line[..mstart], FontId::Mono, FONT_PX).0;
                let mw = fb.measure_text(&find_q, FontId::Mono, FONT_PX).0;
                fb.fill_rect(xoff + GUT_W + 6 + px, y, mw, LH, ED_MATCH);
                from = mstart + find_q.len();
            }
        }
        // syntax-highlighted text
        let mut x = xoff + GUT_W + 6;
        for (span, col) in highlight_line(line, lang) {
            fb.draw_text(x, y, &span, FontId::Mono, FONT_PX, col);
            x += fb.measure_text(&span, FontId::Mono, FONT_PX).0;
        }
    }
    // Cursor.
    if cur_line >= scroll && cur_line < scroll + rows {
        let y = top + 2 + (cur_line - scroll) * LH;
        let line = lines.get(cur_line).copied().unwrap_or("");
        let prefix: String = line.chars().take(cur_col).collect();
        let cx = xoff + GUT_W + 6 + fb.measure_text(&prefix, FontId::Mono, FONT_PX).0;
        fb.fill_rect(cx, y, 2, LH - 1, ED_FG);
    }
    // Status bar (+ find/replace/goto bar).
    let sy = ch - 18;
    fb.fill_rect(0, sy, cw, 18, 0xff007a_cc);
    let label = match bar {
        Bar::Find => format!("Find: {find_q}_"),
        Bar::Goto => format!("Go to line: {find_q}_"),
        Bar::Replace => {
            let (a, b) = if bar_field == 0 { ("_", "") } else { ("", "_") };
            format!("Find: {find_q}{a}   Replace: {replace_q}{b}   (Tab to switch, Enter=all)")
        }
        Bar::None => format!("Ln {}, Col {}    {} bytes    {}", cur_line + 1, cur_col + 1, text.len(), lang),
    };
    fb.draw_text(8, sy + 2, &label, FontId::Ui, 12, 0xffffffff);
}

fn line_start_of(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn toolbar(fb: &Framebuffer, cw: usize, file: &str, status: &str) {
    fb.fill_rect(0, 0, cw, TOOLBAR_H as usize, 0xffc8_ccd4);
    fb.draw_string(6, 6, &format!("{file}  [{status}]"), 0xff30_3840, None);
    fb.fill_rect(cw - 100, 2, 44, 24, 0xffb0_c8e0);
    fb.draw_string(cw - 94, 6, "LOD", 0xff20_4080, None);
    fb.fill_rect(cw - 52, 2, 44, 24, 0xffb0_e0b8);
    fb.draw_string(cw - 46, 6, "SAV", 0xff20_6030, None);
}

// --- syntax highlighting ------------------------------------------------------

fn editor_lang(file: &str) -> &'static str {
    let f = file.to_ascii_uppercase();
    if f.ends_with(".RS") {
        "rust"
    } else if f.ends_with(".C") || f.ends_with(".H") {
        "c"
    } else if f.ends_with(".PY") {
        "py"
    } else if f.ends_with(".JS") {
        "js"
    } else if f.ends_with(".JSON") || f.ends_with(".JSN") {
        "json"
    } else if f.ends_with(".HTM") || f.ends_with(".HTML") {
        "html"
    } else if f.ends_with(".CSS") {
        "css"
    } else if f.ends_with(".SH") {
        "sh"
    } else if f.ends_with(".MD") {
        "md"
    } else {
        "text"
    }
}

fn is_keyword(word: &str, lang: &str) -> bool {
    const RUST: &[&str] = &["fn", "let", "mut", "pub", "struct", "enum", "impl", "for", "while", "loop",
        "if", "else", "match", "return", "use", "mod", "const", "static", "self", "Self", "trait", "where",
        "as", "in", "ref", "move", "unsafe", "async", "await", "type", "dyn", "crate", "super", "true", "false"];
    const C: &[&str] = &["int", "char", "void", "long", "short", "unsigned", "signed", "float", "double",
        "struct", "union", "enum", "typedef", "const", "static", "extern", "return", "if", "else", "for",
        "while", "do", "switch", "case", "break", "continue", "sizeof", "include", "define"];
    const PY: &[&str] = &["def", "class", "import", "from", "if", "else", "elif", "for", "while", "return",
        "self", "None", "True", "False", "and", "or", "not", "in", "is", "lambda", "with", "as", "try", "except", "pass", "yield"];
    const JS: &[&str] = &["function", "var", "let", "const", "if", "else", "for", "while", "return", "class",
        "new", "this", "typeof", "instanceof", "null", "undefined", "true", "false", "async", "await", "import", "export"];
    const SH: &[&str] = &["if", "then", "else", "fi", "for", "while", "do", "done", "case", "esac", "echo",
        "export", "return", "function", "in"];
    let set: &[&str] = match lang {
        "rust" => RUST,
        "c" => C,
        "py" => PY,
        "js" | "html" | "css" | "json" => JS,
        "sh" => SH,
        _ => &[],
    };
    set.contains(&word)
}

fn highlight_line(line: &str, lang: &str) -> Vec<(String, u32)> {
    if lang == "text" {
        return alloc::vec![(line.to_string(), ED_FG)];
    }
    if lang == "md" {
        let col = if line.trim_start().starts_with('#') { ED_KW } else if line.trim_start().starts_with(['-', '*', '>']) { ED_NUM } else { ED_FG };
        return alloc::vec![(line.to_string(), col)];
    }
    let comment = if lang == "py" || lang == "sh" { "#" } else { "//" };
    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if line[byte_at(line, i)..].starts_with(comment) {
            spans.push((chars[i..].iter().collect(), ED_COMMENT));
            break;
        }
        if c == '"' || c == '\'' {
            let q = c;
            let mut s = String::new();
            s.push(c);
            i += 1;
            while i < chars.len() {
                s.push(chars[i]);
                if chars[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push((s, ED_STR));
            continue;
        }
        if c.is_ascii_digit() {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                s.push(chars[i]);
                i += 1;
            }
            spans.push((s, ED_NUM));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                s.push(chars[i]);
                i += 1;
            }
            let col = if is_keyword(&s, lang) { ED_KW } else { ED_FG };
            spans.push((s, col));
            continue;
        }
        spans.push((c.to_string(), ED_FG));
        i += 1;
    }
    spans
}

fn byte_at(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(s.len())
}
