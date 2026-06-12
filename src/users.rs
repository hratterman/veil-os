//! Multi-user accounts for Veil. A user registry persisted to `/etc/users` in
//! the VeilFS, with `useradd`/`userdel`/`su`, a per-user `/home/<name>` seeded
//! with dotfiles (`.profile`/`.veilrc`/`.history`), and helpers for the shell
//! prompt (`user@veil:~$`), shell history, and `.veilrc` preferences (read by
//! the Settings app). A login screen is shown on boot when >1 account exists.

use crate::vfs;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;

const REGISTRY: &str = "/etc/users";

static mut CURRENT: Option<String> = None;

/// The currently logged-in user (defaults to USER.TXT / "guest").
pub fn current() -> String {
    unsafe {
        let p = core::ptr::addr_of_mut!(CURRENT);
        if (*p).is_none() {
            *p = Some(vfs::current_username());
        }
        (*p).clone().unwrap()
    }
}

fn set_current(name: &str) {
    unsafe { *core::ptr::addr_of_mut!(CURRENT) = Some(name.to_string()) };
}

pub fn home_of(name: &str) -> String {
    format!("/home/{name}")
}

/// The list of registered usernames (from `/etc/users`, one per line).
pub fn list() -> Vec<String> {
    let fs = vfs::get();
    match fs.read(REGISTRY) {
        Some(data) => String::from_utf8_lossy(&data)
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect(),
        None => Vec::new(),
    }
}

fn save_list(users: &[String]) {
    let body = users.join("\n");
    let _ = vfs::get().write(REGISTRY, body.as_bytes());
}

pub fn exists(name: &str) -> bool {
    list().iter().any(|u| u == name)
}

/// Create a user: register it and seed `/home/<name>/` with dotfiles.
pub fn useradd(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains(char::is_whitespace) {
        return Err(format!("useradd: invalid name '{name}'"));
    }
    if exists(name) {
        return Err(format!("useradd: user '{name}' already exists"));
    }
    let mut users = list();
    users.push(name.to_string());
    save_list(&users);
    seed_home(name);
    let _ = vfs::get().persist();
    Ok(())
}

/// Seed a user's home directory + default dotfiles (idempotent).
pub fn seed_home(name: &str) {
    let fs = vfs::get();
    let home = home_of(name);
    fs.mkdir_p(&home);
    if fs.read(&format!("{home}/.profile")).is_none() {
        let _ = fs.write(&format!("{home}/.profile"), b"# ~/.profile - sourced on shell start\nexport PATH=/bin:/usr/bin\n");
    }
    if fs.read(&format!("{home}/.veilrc")).is_none() {
        let _ = fs.write(&format!("{home}/.veilrc"), b"theme=dark\nwallpaper=sunset\n");
    }
    if fs.read(&format!("{home}/.history")).is_none() {
        let _ = fs.write(&format!("{home}/.history"), b"");
    }
}

/// Remove a user from the registry (the home directory is left in place unless
/// `purge`). The last remaining user cannot be deleted.
pub fn userdel(name: &str, purge: bool) -> Result<(), String> {
    let mut users = list();
    if !users.iter().any(|u| u == name) {
        return Err(format!("userdel: no such user '{name}'"));
    }
    if users.len() <= 1 {
        return Err("userdel: cannot remove the last account".to_string());
    }
    users.retain(|u| u != name);
    save_list(&users);
    if purge {
        let home = home_of(name);
        let fs = vfs::get();
        if let Some(entries) = fs.ls(&home) {
            for (f, _, _) in entries {
                let _ = fs.remove(&format!("{home}/{f}"));
            }
        }
        let _ = fs.remove(&home);
    }
    let _ = vfs::get().persist();
    Ok(())
}

/// Switch the active user (must exist). Updates the VFS cwd to their home.
pub fn su(name: &str) -> Result<(), String> {
    if !exists(name) {
        return Err(format!("su: user '{name}' does not exist"));
    }
    set_current(name);
    let _ = vfs::get().cd(&home_of(name));
    Ok(())
}

/// True when a login screen should be shown on boot (>1 account registered).
pub fn login_required() -> bool {
    list().len() > 1
}

// ---- .veilrc preferences ---------------------------------------------------

/// Read a preference key from the current user's `.veilrc` (key=value lines).
pub fn pref_get(key: &str) -> Option<String> {
    pref_get_for(&current(), key)
}

pub fn pref_get_for(user: &str, key: &str) -> Option<String> {
    let data = vfs::get().read(&format!("{}/.veilrc", home_of(user)))?;
    for line in String::from_utf8_lossy(&data).lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Set a preference key in the current user's `.veilrc`.
pub fn pref_set(key: &str, val: &str) {
    let user = current();
    let path = format!("{}/.veilrc", home_of(&user));
    let fs = vfs::get();
    let mut lines: Vec<(String, String)> = Vec::new();
    if let Some(data) = fs.read(&path) {
        for line in String::from_utf8_lossy(&data).lines() {
            if let Some((k, v)) = line.split_once('=') {
                lines.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    if let Some(e) = lines.iter_mut().find(|(k, _)| k == key) {
        e.1 = val.to_string();
    } else {
        lines.push((key.to_string(), val.to_string()));
    }
    let body: String = lines.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
    let _ = fs.write(&path, body.as_bytes());
    let _ = fs.persist();
}

// ---- shell history ---------------------------------------------------------

pub fn history_path(user: &str) -> String {
    format!("{}/.history", home_of(user))
}

/// Append a command to the current user's history file.
pub fn history_append(cmd: &str) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }
    let user = current();
    let path = history_path(&user);
    let fs = vfs::get();
    let mut data = fs.read(&path).unwrap_or_default();
    data.extend_from_slice(cmd.as_bytes());
    data.push(b'\n');
    // cap history to the last 200 lines
    let text = String::from_utf8_lossy(&data);
    let lines: Vec<&str> = text.lines().collect();
    let kept = if lines.len() > 200 { &lines[lines.len() - 200..] } else { &lines[..] };
    let _ = fs.write(&path, (kept.join("\n") + "\n").as_bytes());
}

pub fn history_load(user: &str) -> Vec<String> {
    vfs::get()
        .read(&history_path(user))
        .map(|d| String::from_utf8_lossy(&d).lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

/// The shell prompt for the current user + cwd, with `~` for the home dir.
pub fn prompt() -> String {
    let user = current();
    let cwd = vfs::get().cwd_path();
    let home = home_of(&user);
    let display = if cwd == home {
        "~".to_string()
    } else if let Some(rest) = cwd.strip_prefix(&format!("{home}/")) {
        format!("~/{rest}")
    } else {
        cwd
    };
    format!("{user}@veil:{display}$ ")
}

// ---- self-test -------------------------------------------------------------

pub fn selftest() {
    vfs::get(); // ensure the VFS exists (seeded /home/<guest>)

    // Register two users; each gets a home + dotfiles.
    let _ = useradd("alice");
    let _ = useradd("bob");
    let registered = exists("alice") && exists("bob");
    let homes = vfs::get().resolve("/home/alice").is_some() && vfs::get().resolve("/home/bob").is_some();
    let dotfiles = vfs::get().read("/home/alice/.profile").is_some()
        && vfs::get().read("/home/alice/.veilrc").is_some()
        && vfs::get().read("/home/alice/.history").is_some();

    // Switch user; the prompt + cwd follow.
    let _ = su("alice");
    let cur_ok = current() == "alice";
    let prompt_ok = prompt() == "alice@veil:~$ ";

    // .veilrc preferences round-trip.
    pref_set("theme", "light");
    pref_set("wallpaper", "ocean");
    let prefs_ok = pref_get("theme").as_deref() == Some("light")
        && pref_get("wallpaper").as_deref() == Some("ocean");

    // Shell history append + load.
    history_append("ls -la");
    history_append("cd /tmp");
    let hist = history_load("alice");
    let hist_ok = hist.iter().any(|l| l == "ls -la") && hist.iter().any(|l| l == "cd /tmp");

    // Login screen required once >1 account exists.
    let login_ok = login_required();

    // userdel removes from the registry; can't remove the last account.
    let del_ok = userdel("bob", false).is_ok() && !exists("bob");

    // Persistence: the registry survives a fresh VFS load from disk.
    let _ = vfs::get().persist();
    let survives = vfs::Vfs::load()
        .map(|f| {
            let users = f.read("/etc/users").map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default();
            users.contains("alice")
        })
        .unwrap_or(false);

    crate::kprintln!(
        "USERS: registered={registered} homes={homes} dotfiles={dotfiles} cur={cur_ok} prompt={prompt_ok} prefs={prefs_ok} hist={hist_ok} login={login_ok} del={del_ok} survives={survives}"
    );
    let ok = registered && homes && dotfiles && cur_ok && prompt_ok && prefs_ok && hist_ok && login_ok && del_ok && survives;
    if ok {
        crate::kprintln!("USERS_OK: multi-user accounts — useradd/userdel/su, per-user /home + dotfiles, user@veil:~$ prompt, .veilrc prefs, shell history, login-on-multi-user, all persisted to disk");
    } else {
        crate::kprintln!("USERS_FAIL: registered={registered} homes={homes} prompt={prompt_ok} prefs={prefs_ok} hist={hist_ok} survives={survives}");
    }
    // restore the default session user for the rest of boot
    let _ = su("guest").or_else(|_| { let _ = useradd("guest"); su("guest") });
}
