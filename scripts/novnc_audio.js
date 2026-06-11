// Veil OS noVNC audio client (M28). Loaded by the hosted demo's noVNC page;
// streams PCM from the audio bridge (scripts/audio_server.js) and plays it
// through the Web Audio API. Deploy: copy into ~/server/novnc/ and add
//   <script>window.VEIL_SESSION="<id>";</script>
//   <script src="audio.js"></script>
// to index.html (the session manager injects the id at serve time).
(function () {
  "use strict";
  var SR = 44100, CH = 2;

  // Session id: explicit global, else from /session/<id>/... path, else query.
  function sessionId() {
    if (window.VEIL_SESSION) return window.VEIL_SESSION;
    var m = location.pathname.match(/session\/([^/]+)/);
    if (m) return m[1];
    return new URLSearchParams(location.search).get("session") || "";
  }

  var host = "audio.henryratterman.com";
  var url = "wss://" + host + "/?session=" + encodeURIComponent(sessionId());
  // Local dev fallback (page served over http on the bridge host directly).
  if (location.protocol === "http:") {
    url = "ws://" + location.hostname + ":6092/?session=" +
      encodeURIComponent(sessionId());
  }

  var ctx = null, playHead = 0, muted = false, ws = null;
  var icon = makeIcon();

  function ensureContext() {
    if (ctx) return;
    var AC = window.AudioContext || window.webkitAudioContext;
    ctx = new AC({ sampleRate: SR });
    playHead = ctx.currentTime;
  }

  // Browser autoplay policy: only start audio after a user gesture.
  function unlock() {
    ensureContext();
    if (ctx.state === "suspended") ctx.resume();
  }
  window.addEventListener("mousedown", unlock, { once: false });
  window.addEventListener("keydown", unlock, { once: false });

  function playPcm(buf) {
    if (!ctx || muted) return;
    var pcm = new Int16Array(buf);
    var frames = pcm.length / CH;
    if (frames < 1) return;
    var ab = ctx.createBuffer(CH, frames, SR);
    for (var c = 0; c < CH; c++) {
      var out = ab.getChannelData(c);
      for (var i = 0; i < frames; i++) out[i] = pcm[i * CH + c] / 32768;
    }
    var src = ctx.createBufferSource();
    src.buffer = ab;
    src.connect(ctx.destination);
    // Queue back-to-back; if we've fallen behind, resync to now.
    var now = ctx.currentTime;
    if (playHead < now) playHead = now;
    src.start(playHead);
    playHead += ab.duration;
  }

  function connect() {
    setIcon("gray");
    ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    ws.onmessage = function (ev) {
      if (ev.data instanceof ArrayBuffer) { setIcon("green"); playPcm(ev.data); }
    };
    ws.onerror = function () { setIcon("red"); };
    ws.onclose = function () { setIcon("gray"); setTimeout(connect, 2000); };
  }

  function makeIcon() {
    var d = document.createElement("div");
    d.title = "Veil audio (click to mute)";
    d.style.cssText =
      "position:fixed;right:10px;bottom:10px;width:28px;height:28px;" +
      "border-radius:50%;background:#888;z-index:99999;cursor:pointer;" +
      "box-shadow:0 0 4px #000;font:16px/28px monospace;text-align:center;color:#fff";
    d.textContent = "♪";
    d.onclick = function () {
      muted = !muted;
      d.style.opacity = muted ? "0.4" : "1";
      if (!muted) unlock();
    };
    (document.body || document.documentElement).appendChild(d);
    return d;
  }
  function setIcon(state) {
    icon.style.background =
      state === "green" ? "#3ac06a" : state === "red" ? "#c04040" : "#888";
  }

  if (document.readyState === "loading")
    document.addEventListener("DOMContentLoaded", connect);
  else connect();
})();
