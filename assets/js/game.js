// Star Dodger — a complete arcade game for Veil. Canvas 2D rendering, a
// requestAnimationFrame game loop (60fps target), keyboard input, Web Audio
// sound effects, and a high score persisted to localStorage. Installable as a
// .veil package (manifest + this script + game.htm).
(function () {
  'use strict';
  var cv = document.getElementById('game');
  var ctx = cv.getContext('2d');
  var W = cv.width, H = cv.height;

  // --- audio (optional; silent if no device) -----------------------------
  var actx = null;
  function beep(freq, dur) {
    try {
      if (!actx) actx = new AudioContext();
      var o = actx.createOscillator();
      var g = actx.createGain();
      o.frequency.value = freq;
      o.type = 'square';
      g.gain.value = 0.2;
      o.connect(g); g.connect(actx.destination);
      o.start(actx.currentTime);
      o.stop(actx.currentTime + (dur || 0.08));
    } catch (e) {}
  }

  // --- phase -------------------------------------------------------------
  var STATE_START = 0;
  var STATE_PLAY = 1;
  var STATE_OVER = 2;
  var phase = STATE_START;
  var ship = null;
  var stars = null;
  var score = 0;
  var tick = 0;
  var hi = 0;

  hi = parseInt(localStorage.getItem('stardodger_hi') || '0', 10) || 0;

  function reset() {
    ship = { x: W / 2, w: 26, h: 16, speed: 5 };
    stars = [];
    score = 0;
    tick = 0;
  }
  reset();

  // --- input -------------------------------------------------------------
  var keys = {};
  window.addEventListener('keydown', function (e) {
    keys[e.key] = true;
    if (e.key === ' ' || e.key === 'Spacebar' || e.key === 'Enter') {
      if (phase !== STATE_PLAY) { phase = STATE_PLAY; reset(); beep(660, 0.1); }
    }
  });
  window.addEventListener('keyup', function (e) { keys[e.key] = false; });

  function spawnStar() {
    // deterministic-ish spread using the tick counter
    var x = ((tick * 53) % (W - 20)) + 10;
    stars.push({ x: x, y: -10, r: 6 + (tick % 4), v: 2 + (score / 500) });
  }

  function update() {
    tick++;
    if (keys['ArrowLeft']) ship.x -= ship.speed;
    if (keys['ArrowRight']) ship.x += ship.speed;
    if (ship.x < ship.w / 2) ship.x = ship.w / 2;
    if (ship.x > W - ship.w / 2) ship.x = W - ship.w / 2;

    if (tick % 18 === 0) spawnStar();
    var shipY = H - 30;
    for (var i = stars.length - 1; i >= 0; i--) {
      var s = stars[i];
      s.y += s.v;
      // collision with the ship?
      if (s.y > shipY - ship.h && s.y < shipY + ship.h &&
          Math.abs(s.x - ship.x) < ship.w / 2 + s.r) {
        phase = STATE_OVER;
        beep(140, 0.3);
        if (score > hi) { hi = score; localStorage.setItem('stardodger_hi', '' + hi); }
      }
      if (s.y > H + 10) { stars.splice(i, 1); score += 10; } // dodged one
    }
    score += 1; // survival points
    // keep the best score persisted live so it survives a reload
    if (score > hi) { hi = score; localStorage.setItem('stardodger_hi', '' + hi); }
  }

  function draw() {
    ctx.fillStyle = '#05050c';
    ctx.fillRect(0, 0, W, H);

    // falling stars
    ctx.fillStyle = '#ffd84a';
    for (var i = 0; i < stars.length; i++) {
      var s = stars[i];
      ctx.beginPath();
      ctx.arc(s.x, s.y, s.r, 0, 6.2832, false);
      ctx.fill();
    }
    // ship (a triangle)
    var sy = H - 30;
    ctx.fillStyle = '#6ad6ff';
    ctx.beginPath();
    ctx.moveTo(ship.x, sy - ship.h);
    ctx.lineTo(ship.x - ship.w / 2, sy + ship.h);
    ctx.lineTo(ship.x + ship.w / 2, sy + ship.h);
    ctx.fill();

    // HUD
    ctx.fillStyle = '#e8e8f0';
    ctx.font = '16px sans';
    ctx.fillText('Score ' + score, 8, 20);
    ctx.fillText('Best ' + hi, W - 88, 20);

    if (phase === STATE_START) {
      ctx.fillStyle = '#6ad6ff';
      ctx.font = '24px sans';
      ctx.fillText('STAR DODGER', W / 2 - 80, H / 2 - 10);
      ctx.fillStyle = '#9aa';
      ctx.font = '14px sans';
      ctx.fillText('press Space', W / 2 - 44, H / 2 + 16);
    } else if (phase === STATE_OVER) {
      ctx.fillStyle = '#ff5252';
      ctx.font = '24px sans';
      ctx.fillText('GAME OVER', W / 2 - 64, H / 2 - 10);
      ctx.fillStyle = '#9aa';
      ctx.font = '14px sans';
      ctx.fillText('Space to retry', W / 2 - 52, H / 2 + 16);
    }
  }

  function frame() {
    if (phase === STATE_PLAY) update();
    draw();
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  // expose hooks for the self-test harness (all closures over this scope).
  window.__game = {
    start: function () { reset(); phase = STATE_PLAY; },
    // run n game ticks synchronously (update when playing, always draw).
    step: function (n) {
      var i = 0;
      while (i < n) {
        if (phase === STATE_PLAY) { update(); }
        draw();
        i = i + 1;
      }
    },
    // a single summary string built inside this scope (string-first concat).
    summary: function () { return '' + score + ',' + phase + ',' + hi; },
  };
})();
