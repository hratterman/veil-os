//! M35 real shell: commands that operate on the FAT16 disk — ls/cat/cp/mv/rm,
//! echo with `>` redirection, two-stage pipes (`cat f | grep x`), pwd/cd, and
//! `run <app>` to launch a GUI app. The window/REPL plumbing (history, tab
//! completion, rendering) lives in wm.rs; this module is the command engine.

use crate::fs;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const HELP: &str = "veil shell commands:\n  \
ls [-l]           list files (with sizes)\n  \
cat <file>        print a file\n  \
cp <src> <dst>    copy a file\n  \
mv <src> <dst>    rename/move a file\n  \
rm <file>         delete a file\n  \
echo <s> [> f]    print text or write it to a file\n  \
grep <pat>        filter piped lines\n  \
pwd / cd          working directory (root-only FS)\n  \
run <app>         launch browser/viewer/lisp/files/...\n  \
clear / help\n";

/// What a command line produced: text to print and an optional app to launch.
pub struct Outcome {
    pub out: String,
    pub launch: Option<String>,
    pub clear: bool,
}

/// Run a full command line (supports `|` pipes and `>` redirection).
pub fn run(line: &str) -> Outcome {
    let line = line.trim();
    if line.is_empty() {
        return Outcome { out: String::new(), launch: None, clear: false };
    }
    if line == "clear" {
        return Outcome { out: String::new(), launch: None, clear: true };
    }
    let mut out = String::new();
    let mut launch = None;
    // Sequence by ';' (always run), then chain by && / || within each segment.
    for seg in line.split(';') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        let mut last_ok = true;
        for (op, cmd) in split_andor(seg) {
            let do_it = match op {
                "&&" => last_ok,
                "||" => !last_ok,
                _ => true,
            };
            if !do_it {
                continue;
            }
            let (o, l, ok) = run_pipeline(cmd.trim());
            out.push_str(&o);
            launch = launch.or(l);
            last_ok = ok;
        }
    }
    Outcome { out, launch, clear: false }
}

/// Split "a && b || c" into [("", "a"), ("&&", "b"), ("||", "c")].
fn split_andor(s: &str) -> Vec<(&str, &str)> {
    let mut res = Vec::new();
    let (mut start, mut prev_op, mut i) = (0usize, "", 0usize);
    let b = s.as_bytes();
    while i + 1 < b.len() {
        if &s[i..i + 2] == "&&" || &s[i..i + 2] == "||" {
            res.push((prev_op, s[start..i].trim()));
            prev_op = &s[i..i + 2];
            i += 2;
            start = i;
        } else {
            i += 1;
        }
    }
    res.push((prev_op, s[start..].trim()));
    res
}

/// Run a pipeline `cmd1 | cmd2 | ...`; returns (output, launch, success).
fn run_pipeline(pipeline: &str) -> (String, Option<String>, bool) {
    let stages: Vec<&str> = pipeline.split('|').map(str::trim).collect();
    let mut stdin: Option<String> = None;
    let mut launch = None;
    let mut out = String::new();
    let mut ok = true;
    for (i, stage) in stages.iter().enumerate() {
        let r = run_stage(stage, stdin.take());
        launch = launch.or(r.launch);
        ok = r.ok;
        if i + 1 < stages.len() {
            stdin = Some(r.out);
        } else {
            out = r.out;
        }
    }
    (out, launch, ok)
}

struct StageOut {
    out: String,
    launch: Option<String>,
    ok: bool,
}

fn run_stage(stage: &str, stdin: Option<String>) -> StageOut {
    // Output redirection: `cmd ... > file`.
    let (cmd, redirect) = match stage.split_once('>') {
        Some((c, f)) => (c.trim(), Some(f.trim().to_string())),
        None => (stage, None),
    };
    let (name, args) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
    let args = args.trim();
    let mut launch = None;
    let out = match name {
        "ls" => ls(args),
        "cat" => cat(args),
        "cp" => cp(args),
        "mv" => mv(args),
        "rm" => rm(args),
        "mkdir" => "mkdir: this disk is FAT16 root-only (no subdirectories)\n".to_string(),
        "echo" => format!("{args}\n"),
        "pwd" => "/\n".to_string(),
        "cd" => cd(args),
        "grep" => grep(args, stdin.as_deref()),
        "head" => head(args, stdin.as_deref()),
        "tail" => tail(args, stdin.as_deref()),
        "wc" => wc(args, stdin.as_deref()),
        "sort" => sort(args, stdin.as_deref()),
        "find" => find(args),
        "date" => date(),
        "df" => df(),
        "env" => env_list(),
        "export" => env_set(args),
        "chmod" => String::new(), // permissions are faked; never error
        "run" => {
            launch = Some(args.to_ascii_lowercase());
            format!("launching {args}...\n")
        }
        "help" => HELP.to_string(),
        "" => String::new(),
        other => format!("{other}: command not found\n"),
    };
    if let Some(f) = redirect {
        return match fs::write_file(&f, out.as_bytes()) {
            Ok(()) => StageOut { out: String::new(), launch, ok: true },
            Err(()) => StageOut { out: format!("{f}: write failed (disk full / bad name?)\n"), launch, ok: false },
        };
    }
    let ok = !is_error(&out);
    StageOut { out, launch, ok }
}

/// Heuristic exit status from a command's output (for && / ||). Matches only
/// our own "cmd: ... error" message shapes, not arbitrary file contents.
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
    match fs::read_file(args) {
        Some(data) => {
            let mut s = String::from_utf8_lossy(&data).into_owned();
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s
        }
        None => format!("cat: {args}: no such file\n"),
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
    match fs::delete(args) {
        Ok(()) => String::new(),
        Err(()) => format!("rm: {args}: no such file\n"),
    }
}

fn cd(args: &str) -> String {
    match args.trim() {
        "" | "/" | "." => String::new(),
        other => format!("cd: {other}: root-only filesystem\n"),
    }
}

/// Resolve a command's input: piped stdin if present, else the named file.
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
    let (pat, file) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
    let text = input_text(file, stdin);
    text.lines().filter(|l| l.contains(pat.trim())).map(|l| format!("{l}\n")).collect()
}

fn nlines(args: &str) -> (usize, &str) {
    // parse "-n N rest" / "rest"
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
    let text = input_text(args, stdin);
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort();
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn wc(args: &str, stdin: Option<&str>) -> String {
    let flag = args.split_whitespace().next().filter(|a| a.starts_with('-')).unwrap_or("");
    let s = input_text(args, stdin);
    let (l, w, c) = (s.lines().count(), s.split_whitespace().count(), s.len());
    match flag {
        "-l" => format!("{l}\n"),
        "-w" => format!("{w}\n"),
        "-c" => format!("{c}\n"),
        _ => format!("{l} {w} {c}\n"),
    }
}

fn find(args: &str) -> String {
    // find <path> -name <pattern>   (glob: * matches any suffix)
    let pat = args.split_once("-name").map(|(_, p)| p.trim()).unwrap_or("*");
    let stem = pat.trim_matches('*').to_ascii_uppercase();
    let mut out = String::new();
    for (name, _) in fs::list_root().unwrap_or_default() {
        let m = if pat.starts_with('*') && pat.ends_with('*') {
            name.contains(&stem)
        } else if let Some(ext) = pat.strip_prefix('*') {
            name.ends_with(&ext.to_ascii_uppercase())
        } else {
            name.eq_ignore_ascii_case(pat)
        };
        if m || pat == "*" {
            out.push_str(&format!("/{name}\n"));
        }
    }
    out
}

fn date() -> String {
    let secs = fs::read_file("TZ.TXT"); // unused placeholder to keep fs import
    let _ = secs;
    let unix = crate::timer::wall_ticks50().map(|t| t / 50);
    match unix {
        Some(s) => format!("{}\n", civil(s as i64)),
        None => format!("uptime {}s (no NTP sync)\n", crate::timer::uptime_secs()),
    }
}

/// Convert a Unix timestamp to "YYYY-MM-DD HH:MM:SS UTC" (Hinnant's algorithm).
fn civil(t: i64) -> alloc::string::String {
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

// A tiny in-memory environment (PATH-style), persisted only for the session.
static mut ENV: Option<Vec<(String, String)>> = None;
fn env_vars() -> &'static mut Vec<(String, String)> {
    unsafe {
        let e = &mut *core::ptr::addr_of_mut!(ENV);
        if e.is_none() {
            *e = Some(alloc::vec![("USER".to_string(), "guest".to_string()), ("SHELL".to_string(), "/bin/vsh".to_string())]);
        }
        e.as_mut().unwrap()
    }
}
fn env_list() -> String {
    env_vars().iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}
fn env_set(args: &str) -> String {
    if let Some((k, v)) = args.split_once('=') {
        let (k, v) = (k.trim().to_string(), v.trim().to_string());
        let e = env_vars();
        if let Some(slot) = e.iter_mut().find(|(ek, _)| *ek == k) {
            slot.1 = v;
        } else {
            e.push((k, v));
        }
    }
    String::new()
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
