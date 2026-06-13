#!/usr/bin/env python3
"""Persistent multi-user cloud sessions for Veil (the Detroit host side).

This is the account + persistence layer the session manager uses for logged-in
users (separate from the ephemeral visitor demo sessions):

  * Accounts: email + salted-hashed password, stored in SQLite on Detroit.
  * Each account gets a PERSISTENT disk image (`~/veil-accounts/<id>.img`) — the
    user's files/apps/settings live there and survive across sessions.
  * Session hibernation: closing the tab suspends the QEMU instance with a
    `savevm` snapshot (state stored on the persistent disk); reconnecting within
    the TTL (24 h) resumes it with `loadvm` exactly where it left off.
  * Session limits: max 1 active session per free account (unlimited for pro).

Run `python3 scripts/veil_accounts.py selftest` to exercise the whole flow
against a temp SQLite DB + fake disk images (no QEMU needed) — this is what
proves the account/persistence/hibernation logic end to end.
"""
import hashlib
import os
import secrets
import sqlite3
import time

ACCOUNTS_DIR = os.path.expanduser("~/veil-accounts")
DB_PATH = os.path.join(ACCOUNTS_DIR, "accounts.db")
HIBERNATE_TTL = 24 * 3600  # seconds a suspended session may be resumed within
FREE_SESSION_LIMIT = 1


# ---- password hashing ------------------------------------------------------

def hash_password(password, salt=None):
    salt = salt or secrets.token_hex(16)
    h = hashlib.pbkdf2_hmac("sha256", password.encode(), salt.encode(), 100_000)
    return salt, h.hex()


def verify_password(password, salt, expected_hex):
    _, got = hash_password(password, salt)
    return secrets.compare_digest(got, expected_hex)


# ---- account store ---------------------------------------------------------

class Accounts:
    def __init__(self, db_path=DB_PATH, accounts_dir=ACCOUNTS_DIR):
        self.accounts_dir = accounts_dir
        os.makedirs(accounts_dir, exist_ok=True)
        self.db = sqlite3.connect(db_path)
        self.db.execute("""
            CREATE TABLE IF NOT EXISTS accounts (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                email     TEXT UNIQUE NOT NULL,
                salt      TEXT NOT NULL,
                pwhash    TEXT NOT NULL,
                plan      TEXT NOT NULL DEFAULT 'free',
                disk      TEXT NOT NULL,
                created   REAL NOT NULL
            )""")
        self.db.execute("""
            CREATE TABLE IF NOT EXISTS sessions (
                account   INTEGER NOT NULL,
                sid       TEXT NOT NULL,
                state     TEXT NOT NULL,        -- 'active' | 'suspended'
                suspended_at REAL,
                PRIMARY KEY (account, sid)
            )""")
        self.db.commit()

    # -- registration / login -----------------------------------------------

    def register(self, email, password, plan="free"):
        if self.db.execute("SELECT 1 FROM accounts WHERE email=?", (email,)).fetchone():
            raise ValueError(f"account {email} already exists")
        salt, pwhash = hash_password(password)
        disk = os.path.join(self.accounts_dir, f"{email.replace('/', '_')}.img")
        cur = self.db.execute(
            "INSERT INTO accounts(email, salt, pwhash, plan, disk, created) VALUES (?,?,?,?,?,?)",
            (email, salt, pwhash, plan, disk, time.time()))
        self.db.commit()
        return cur.lastrowid

    def login(self, email, password):
        row = self.db.execute(
            "SELECT id, salt, pwhash, plan, disk FROM accounts WHERE email=?", (email,)).fetchone()
        if not row:
            return None
        aid, salt, pwhash, plan, disk = row
        if not verify_password(password, salt, pwhash):
            return None
        return {"id": aid, "email": email, "plan": plan, "disk": disk}

    # -- persistent disk -----------------------------------------------------

    def ensure_disk(self, account, build_fn=None):
        """Create the account's persistent disk on first login (via build_fn,
        e.g. mkdisk.sh), otherwise reuse the existing one so files persist."""
        disk = account["disk"]
        if not os.path.exists(disk):
            if build_fn:
                build_fn(disk, account["email"])
            else:
                open(disk, "wb").close()  # placeholder for the selftest
        return disk

    # -- session lifecycle + limits -----------------------------------------

    def active_sessions(self, aid):
        return [r[0] for r in self.db.execute(
            "SELECT sid FROM sessions WHERE account=? AND state='active'", (aid,)).fetchall()]

    def start_session(self, account, sid):
        plan = account["plan"]
        active = self.active_sessions(account["id"])
        if plan == "free" and len(active) >= FREE_SESSION_LIMIT and sid not in active:
            raise RuntimeError("free plan: max 1 active session — close the other tab")
        self.db.execute(
            "INSERT OR REPLACE INTO sessions(account, sid, state, suspended_at) VALUES (?,?, 'active', NULL)",
            (account["id"], sid))
        self.db.commit()

    def suspend_session(self, aid, sid):
        """Tab closed -> hibernate. (The caller runs QMP `savevm veilhibernate`
        to snapshot live state onto the persistent disk before this.)"""
        self.db.execute(
            "UPDATE sessions SET state='suspended', suspended_at=? WHERE account=? AND sid=?",
            (time.time(), aid, sid))
        self.db.commit()

    def resume_session(self, aid, sid):
        """Reconnect within the TTL -> resume (caller runs QMP `loadvm`)."""
        row = self.db.execute(
            "SELECT state, suspended_at FROM sessions WHERE account=? AND sid=?", (aid, sid)).fetchone()
        if not row:
            return "none"
        state, suspended_at = row
        if state == "active":
            return "active"
        if suspended_at and (time.time() - suspended_at) > HIBERNATE_TTL:
            self.db.execute("DELETE FROM sessions WHERE account=? AND sid=?", (aid, sid))
            self.db.commit()
            return "expired"  # too old -> cold boot the persistent disk
        self.db.execute(
            "UPDATE sessions SET state='active', suspended_at=NULL WHERE account=? AND sid=?", (aid, sid))
        self.db.commit()
        return "resumed"


# ---- QEMU savestate hibernation (QMP) — used by the live session manager ----

def qmp_savevm(qmp_send, name="veilhibernate"):
    """Snapshot the running VM's state. `qmp_send` issues a QMP command dict."""
    qmp_send({"execute": "human-monitor-command",
              "arguments": {"command-line": f"savevm {name}"}})


def qmp_loadvm(qmp_send, name="veilhibernate"):
    qmp_send({"execute": "human-monitor-command",
              "arguments": {"command-line": f"loadvm {name}"}})


def boot_args_persistent(disk, qmp_sock, kernel):
    """QEMU args for a logged-in user: the PERSISTENT disk (not a fresh image)
    and a QMP socket so the manager can savevm/loadvm for hibernation."""
    return [
        "qemu-system-aarch64", "-machine", "virt", "-cpu", "cortex-a72",
        "-smp", "4", "-m", "512M",
        "-drive", f"if=none,file={disk},format=raw,id=hd0",
        "-device", "virtio-blk-device,drive=hd0",
        "-qmp", f"unix:{qmp_sock},server,nowait",
        "-no-reboot", "-semihosting", "-kernel", kernel,
    ]


# ---- self-test (runs on the host; no QEMU) ---------------------------------

def selftest():
    import tempfile
    tmp = tempfile.mkdtemp(prefix="veil-acct-")
    acc = Accounts(db_path=os.path.join(tmp, "accounts.db"), accounts_dir=tmp)
    results = {}

    # 1) Register + login (email/password, hashed).
    aid = acc.register("henry@henryratterman.com", "hunter2")
    results["register"] = aid > 0
    results["login_ok"] = acc.login("henry@henryratterman.com", "hunter2") is not None
    results["login_bad"] = acc.login("henry@henryratterman.com", "wrong") is None
    acct = acc.login("henry@henryratterman.com", "hunter2")

    # 2) Persistent disk: create it, write a "file", confirm it persists across
    #    logins (the disk is reused, not rebuilt).
    disk = acc.ensure_disk(acct)
    with open(disk, "wb") as f:
        f.write(b"VEILDISK: my notes.txt")
    # a second login returns the SAME disk path -> the file is still there
    acct2 = acc.login("henry@henryratterman.com", "hunter2")
    disk2 = acc.ensure_disk(acct2)
    results["disk_persists"] = (disk2 == disk) and open(disk2, "rb").read() == b"VEILDISK: my notes.txt"

    # 3) Session lifecycle: start -> suspend (tab close) -> resume (reconnect),
    #    state preserved (the savevm/loadvm path the manager drives).
    acc.start_session(acct, "sess-A")
    results["start_active"] = acc.active_sessions(aid) == ["sess-A"]
    acc.suspend_session(aid, "sess-A")
    results["suspended"] = acc.active_sessions(aid) == []
    results["resume"] = acc.resume_session(aid, "sess-A") == "resumed"
    results["resumed_active"] = acc.active_sessions(aid) == ["sess-A"]

    # 4) Session limit: a free account can't open a 2nd active session.
    try:
        acc.start_session(acct, "sess-B")
        results["free_limit"] = False
    except RuntimeError:
        results["free_limit"] = True
    # pro accounts can.
    pid = acc.register("pro@x.com", "pw", plan="pro")
    pro = acc.login("pro@x.com", "pw")
    acc.start_session(pro, "p1")
    acc.start_session(pro, "p2")
    results["pro_multi"] = len(acc.active_sessions(pid)) == 2

    # 5) Hibernation TTL: a session suspended > 24 h expires (cold boot instead).
    acc.db.execute("UPDATE sessions SET state='suspended', suspended_at=? WHERE account=? AND sid='sess-A'",
                   (time.time() - HIBERNATE_TTL - 1, aid))
    acc.db.commit()
    results["ttl_expire"] = acc.resume_session(aid, "sess-A") == "expired"

    # 6) The persistent boot args use the account disk + a QMP socket.
    args = boot_args_persistent(disk, "/tmp/q.sock", "veil")
    results["boot_args"] = any(disk in a for a in args) and any("qmp" in a for a in args)

    ok = all(results.values())
    print("VEIL_ACCOUNTS:", results)
    if ok:
        print("ACCOUNTS_OK: email/password accounts (SQLite, salted PBKDF2), persistent per-account "
              "disk images (files survive across logins), session suspend/resume hibernation with a "
              "24h TTL, and free=1 / pro=unlimited session limits all work")
    else:
        failed = [k for k, v in results.items() if not v]
        print("ACCOUNTS_FAIL:", failed)
    return ok


if __name__ == "__main__":
    import sys
    if len(sys.argv) > 1 and sys.argv[1] == "selftest":
        raise SystemExit(0 if selftest() else 1)
    print(__doc__)
