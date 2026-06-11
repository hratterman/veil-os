// Veil OS noVNC audio client. Loaded by the hosted demo's noVNC page; streams
// 16-bit signed stereo 44100 Hz PCM from the session manager's same-origin
// WebSocket at /session/<id>/audio and plays it through the Web Audio API.
// Deploy: install_sessions.sh copies this to ~/server/novnc/audio.js and adds
//   <script src="audio.js"></script>
// to vnc.html. The session id is read from the /session/<id>/ URL path.
//
// Playback model (M33 fix): the ♪ button is the single control and the ONLY
// reliable user gesture (noVNC's canvas swallows mousedown via
// stopPropagation, so window-level gesture listeners can't be trusted to
// unlock the AudioContext). First click enables audio (creates + resumes the
// context); subsequent clicks toggle mute. Browsers forbid audio before a
// gesture, so we stay silent until that first click. PCM chunks are scheduled
// back-to-back on a play head kept a short lookahead ahead of currentTime so
// the first buffer never starts in the past.
(function () {
  "use strict";
  var SR = 44100, CH = 2, LOOKAHEAD = 0.15;

  function sessionId() {
    if (window.VEIL_SESSION) return window.VEIL_SESSION;
    var m = location.pathname.match(/session\/([^/]+)/);
    if (m) return m[1];
    return new URLSearchParams(location.search).get("session") || "";
  }

  var proto = location.protocol === "https:" ? "wss" : "ws";
  var url = proto + "://" + location.host + "/session/" +
    encodeURIComponent(sessionId()) + "/audio";

  var ctx = null, playHead = 0, audioOn = false, ws = null, bytesPlayed = 0;
  var icon = makeIcon();

  // Exposed for automated tests (drive_audio_browser): lets a headless browser
  // confirm the context is actually running and PCM is being scheduled.
  window.__veilAudio = {
    get state() { return ctx ? ctx.state : "none"; },
    get bytesPlayed() { return bytesPlayed; },
    get scheduledAhead() { return ctx ? playHead - ctx.currentTime : 0; },
    get on() { return audioOn; },
    enable: enable,
  };

  function ensureContext() {
    if (ctx) return;
    var AC = window.AudioContext || window.webkitAudioContext;
    ctx = new AC({ sampleRate: SR });
    playHead = ctx.currentTime + LOOKAHEAD;
  }

  // Enable audio from within a user-gesture handler (required by autoplay
  // policy). Creates the context, resumes it, unmutes.
  function enable() {
    ensureContext();
    if (ctx.state === "suspended") ctx.resume();
    audioOn = true;
    setIcon();
  }

  function playPcm(buf) {
    if (!audioOn || !ctx) return;
    // Re-resume defensively: browsers can auto-suspend a backgrounded context.
    if (ctx.state === "suspended") ctx.resume();
    var pcm = new Int16Array(buf);
    var frames = (pcm.length / CH) | 0;
    if (frames < 1) return;
    var ab = ctx.createBuffer(CH, frames, SR);
    for (var c = 0; c < CH; c++) {
      var out = ab.getChannelData(c);
      for (var i = 0; i < frames; i++) out[i] = pcm[i * CH + c] / 32768;
    }
    var src = ctx.createBufferSource();
    src.buffer = ab;
    src.connect(ctx.destination);
    var now = ctx.currentTime;
    // If we've underrun (or just started), resync the play head a lookahead
    // beyond now so the buffer is scheduled slightly in the future, not the past.
    if (playHead < now + 0.001) playHead = now + LOOKAHEAD;
    src.start(playHead);
    playHead += ab.duration;
    bytesPlayed += buf.byteLength;
    setIcon();
  }

  function connect() {
    ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    ws.onopen = function () { setIcon(); };
    ws.onmessage = function (ev) {
      if (ev.data instanceof ArrayBuffer) playPcm(ev.data);
    };
    ws.onerror = function () { setIcon(); };
    ws.onclose = function () { ws = null; setTimeout(connect, 2000); };
  }

  function makeIcon() {
    var d = document.createElement("div");
    d.style.cssText =
      "position:fixed;right:10px;bottom:10px;width:30px;height:30px;" +
      "border-radius:50%;z-index:99999;cursor:pointer;box-shadow:0 0 5px #000;" +
      "font:17px/30px monospace;text-align:center;color:#fff;user-select:none";
    d.textContent = "♪";
    d.onclick = function (e) {
      e.stopPropagation();
      if (!audioOn) {
        enable();              // first click: turn audio on
      } else {
        audioOn = false;       // subsequent clicks: mute / unmute
        setIcon();
      }
    };
    (document.body || document.documentElement).appendChild(d);
    return d;
  }

  function setIcon() {
    var connected = ws && ws.readyState === 1;
    if (!audioOn) {
      icon.style.background = "#666";   // off: click to enable
      icon.style.opacity = "0.65";
      icon.title = "Veil audio — click to enable sound";
    } else {
      icon.style.opacity = "1";
      icon.style.background = bytesPlayed > 0 ? "#3ac06a"
        : connected ? "#caa83a" : "#888";   // green=playing, amber=waiting
      icon.title = "Veil audio — on (click to mute)";
    }
  }

  if (document.readyState === "loading")
    document.addEventListener("DOMContentLoaded", connect);
  else connect();
})();
