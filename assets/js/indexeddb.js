// Veil IndexedDB: a small polyfill implementing the subset of the IndexedDB API
// real pages use (open/createObjectStore/transaction/put/add/get/getAll/delete/
// clear), backed by the engine's localStorage (which persists per-origin to the
// FAT16 disk). Requests fire their onsuccess/onupgradeneeded asynchronously via
// setTimeout, draining through the engine's deferred queue — same as a browser.
// Injected ahead of page scripts whenever a page references `indexedDB`.

class IDBRequest {
  constructor() {
    this.onsuccess = null;
    this.onerror = null;
    this.onupgradeneeded = null;
    this.result = undefined;
    this.error = null;
    this.readyState = "pending";
    this.transaction = null;
  }
}

function __idbReq(fn) {
  var r = new IDBRequest();
  setTimeout(function () {
    try {
      r.result = fn();
      r.readyState = "done";
      if (r.onsuccess) r.onsuccess({ target: r });
    } catch (e) {
      r.error = e;
      r.readyState = "done";
      if (r.onerror) r.onerror({ target: r });
    }
  }, 0);
  return r;
}

class IDBObjectStore {
  constructor(db, name) {
    this._db = db;
    this.name = name;
    this.keyPath = __idbKeyPaths[db + ":" + name] || null;
    this._auto = 0;
  }
  _key() { return "__idb:" + this._db + ":" + this.name; }
  _load() {
    try { return JSON.parse(localStorage.getItem(this._key()) || "{}"); }
    catch (e) { return {}; }
  }
  _save(o) { localStorage.setItem(this._key(), JSON.stringify(o)); }
  put(value, key) {
    var self = this;
    return __idbReq(function () {
      var d = self._load();
      var k = key;
      if (k === undefined && self.keyPath) k = value[self.keyPath];
      if (k === undefined) k = "auto_" + (Object.keys(d).length + 1);
      d["" + k] = value;
      self._save(d);
      return k;
    });
  }
  add(value, key) { return this.put(value, key); }
  get(key) {
    var self = this;
    return __idbReq(function () { return self._load()["" + key]; });
  }
  getAll() {
    var self = this;
    return __idbReq(function () {
      var d = self._load();
      return Object.keys(d).map(function (k) { return d[k]; });
    });
  }
  getAllKeys() {
    var self = this;
    return __idbReq(function () { return Object.keys(self._load()); });
  }
  delete(key) {
    var self = this;
    return __idbReq(function () {
      var d = self._load();
      delete d["" + key];
      self._save(d);
    });
  }
  clear() {
    var self = this;
    return __idbReq(function () { self._save({}); });
  }
  count() {
    var self = this;
    return __idbReq(function () { return Object.keys(self._load()).length; });
  }
  createIndex() { return {}; }
}

class IDBTransaction {
  constructor(db, names, mode) {
    this._db = db;
    this.mode = mode || "readonly";
    this.oncomplete = null;
    this.onerror = null;
    this.onabort = null;
    var self = this;
    // Fire oncomplete after this turn's queued requests have drained.
    setTimeout(function () { if (self.oncomplete) self.oncomplete({ target: self }); }, 0);
  }
  objectStore(name) { return new IDBObjectStore(this._db, name); }
  abort() {}
}

class IDBDatabase {
  constructor(name) {
    this.name = name;
    this.version = parseInt(localStorage.getItem("__idbver:" + name) || "1");
    this.objectStoreNames = __idbStoreNames(name);
  }
  createObjectStore(name, opts) {
    __idbKeyPaths[this.name + ":" + name] = opts && opts.keyPath ? opts.keyPath : null;
    var list = __idbStoreNames(this.name);
    if (list.indexOf(name) < 0) {
      list.push(name);
      localStorage.setItem("__idbstores:" + this.name, JSON.stringify(list));
    }
    this.objectStoreNames = list;
    // Ensure the backing record exists.
    if (!localStorage.getItem("__idb:" + this.name + ":" + name)) {
      localStorage.setItem("__idb:" + this.name + ":" + name, "{}");
    }
    return new IDBObjectStore(this.name, name);
  }
  deleteObjectStore(name) {
    localStorage.removeItem("__idb:" + this.name + ":" + name);
  }
  transaction(names, mode) { return new IDBTransaction(this.name, names, mode); }
  close() {}
}

var __idbKeyPaths = {};
function __idbStoreNames(db) {
  try { return JSON.parse(localStorage.getItem("__idbstores:" + db) || "[]"); }
  catch (e) { return []; }
}

var indexedDB = {
  open: function (name, version) {
    var req = new IDBRequest();
    var db = new IDBDatabase(name);
    req.result = db;
    setTimeout(function () {
      var verKey = "__idbver:" + name;
      var old = parseInt(localStorage.getItem(verKey) || "0");
      var want = version || 1;
      if (want > old) {
        localStorage.setItem(verKey, "" + want);
        db.version = want;
        if (req.onupgradeneeded) {
          req.onupgradeneeded({ target: req, oldVersion: old, newVersion: want });
        }
      }
      req.readyState = "done";
      if (req.onsuccess) req.onsuccess({ target: req });
    }, 0);
    return req;
  },
  deleteDatabase: function (name) {
    return __idbReq(function () {
      var names = __idbStoreNames(name);
      for (var i = 0; i < names.length; i++) {
        localStorage.removeItem("__idb:" + name + ":" + names[i]);
      }
      localStorage.removeItem("__idbstores:" + name);
      localStorage.removeItem("__idbver:" + name);
    });
  },
};
window.indexedDB = indexedDB;
