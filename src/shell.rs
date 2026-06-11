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
    let stages: Vec<&str> = line.split('|').map(str::trim).collect();
    let mut stdin: Option<String> = None;
    let mut launch = None;
    let mut out = String::new();
    for (i, stage) in stages.iter().enumerate() {
        let r = run_stage(stage, stdin.take());
        launch = launch.or(r.launch);
        if i + 1 < stages.len() {
            stdin = Some(r.out);
        } else {
            out = r.out;
        }
    }
    Outcome { out, launch, clear: false }
}

struct StageOut {
    out: String,
    launch: Option<String>,
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
        "wc" => wc(stdin.as_deref()),
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
            Ok(()) => StageOut { out: String::new(), launch },
            Err(()) => StageOut { out: format!("{f}: write failed (disk full / bad name?)\n"), launch },
        };
    }
    StageOut { out, launch }
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

fn grep(pat: &str, stdin: Option<&str>) -> String {
    let pat = pat.trim();
    stdin
        .unwrap_or("")
        .lines()
        .filter(|l| l.contains(pat))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn head(args: &str, stdin: Option<&str>) -> String {
    let n = args.trim().trim_start_matches("-n").trim().parse::<usize>().unwrap_or(10);
    stdin.unwrap_or("").lines().take(n).map(|l| format!("{l}\n")).collect()
}

fn wc(stdin: Option<&str>) -> String {
    let s = stdin.unwrap_or("");
    let lines = s.lines().count();
    let words = s.split_whitespace().count();
    format!("{lines} {words} {}\n", s.len())
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
