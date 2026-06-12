// Web Audio API — a from-scratch polyfill for Veil's JS engine. The node graph
// is synthesised in pure JS (Math.sin); on a node's start() the rendered PCM is
// handed to the kernel's virtio-sound driver via __webaudio_play(samples, rate)
// (a no-op when no sound device is present, e.g. headless self-tests).
(function (g) {
  'use strict';

  function AudioParam(v) { this.value = v; }
  AudioParam.prototype.setValueAtTime = function (v) { this.value = v; return this; };
  AudioParam.prototype.linearRampToValueAtTime = function (v) { this.value = v; return this; };
  AudioParam.prototype.exponentialRampToValueAtTime = function (v) { this.value = v; return this; };
  AudioParam.prototype.cancelScheduledValues = function () { return this; };

  function AudioNode(ctx) { this.context = ctx; this._targets = []; }
  AudioNode.prototype.connect = function (dest) { this._targets.push(dest); return dest; };
  AudioNode.prototype.disconnect = function () { this._targets = []; };

  // Effective linear gain from this node down to the destination.
  function gainTo(node, dest, seen) {
    if (node === dest) return 1;
    seen = seen || [];
    if (seen.indexOf(node) >= 0) return 0;
    seen.push(node);
    var total = 0;
    for (var i = 0; i < node._targets.length; i++) {
      var t = node._targets[i];
      var local = (t && t.gain) ? t.gain.value : 1;
      total += local * gainTo(t, dest, seen);
    }
    return total;
  }

  function OscillatorNode(ctx) {
    AudioNode.call(this, ctx);
    this.frequency = new AudioParam(440);
    this.detune = new AudioParam(0);
    this.type = 'sine';
    this.onended = null;
  }
  OscillatorNode.prototype = Object.create(AudioNode.prototype);
  OscillatorNode.prototype.start = function (when) {
    var ctx = this.context;
    var rate = ctx.sampleRate;
    var seconds = 0.25;
    var n = Math.floor(rate * seconds);
    var f = this.frequency.value;
    var amp = gainTo(this, ctx.destination, []);
    var type = this.type;
    var out = new Array(n);
    var twoPi = Math.PI * 2;
    for (var i = 0; i < n; i++) {
      var phase = (twoPi * f * i) / rate;
      var s;
      if (type === 'square') s = (Math.sin(phase) >= 0) ? 1 : -1;
      else if (type === 'sawtooth') { var p = (f * i / rate) % 1; s = 2 * p - 1; }
      else if (type === 'triangle') { var q = (f * i / rate) % 1; s = q < 0.5 ? (4 * q - 1) : (3 - 4 * q); }
      else s = Math.sin(phase); // sine (default)
      out[i] = s * amp;
    }
    ctx._rendered = out;
    ctx._lastFreq = f;
    __webaudio_play(out, rate); // hand to virtio-sound (best-effort)
    if (typeof this.onended === 'function') this.onended();
    return this;
  };
  OscillatorNode.prototype.stop = function () { return this; };

  function GainNode(ctx) { AudioNode.call(this, ctx); this.gain = new AudioParam(1); }
  GainNode.prototype = Object.create(AudioNode.prototype);

  function BufferSourceNode(ctx) { AudioNode.call(this, ctx); this.buffer = null; }
  BufferSourceNode.prototype = Object.create(AudioNode.prototype);
  BufferSourceNode.prototype.start = function () {
    if (this.buffer && this.buffer._data) {
      this.context._rendered = this.buffer._data;
      __webaudio_play(this.buffer._data, this.context.sampleRate);
    }
    return this;
  };
  BufferSourceNode.prototype.stop = function () { return this; };

  function AudioBuffer(rate, data) {
    this.sampleRate = rate;
    this.length = data ? data.length : 0;
    this.duration = this.length / rate;
    this.numberOfChannels = 1;
    this._data = data || [];
  }
  AudioBuffer.prototype.getChannelData = function () { return this._data; };

  function AudioContext() {
    this.sampleRate = 44100;
    this.currentTime = 0;
    this.state = 'running';
    this.destination = new AudioNode(this);
    this._rendered = null;
  }
  AudioContext.prototype.createOscillator = function () { return new OscillatorNode(this); };
  AudioContext.prototype.createGain = function () { return new GainNode(this); };
  AudioContext.prototype.createBufferSource = function () { return new BufferSourceNode(this); };
  AudioContext.prototype.createBuffer = function (ch, len, rate) {
    return new AudioBuffer(rate || this.sampleRate, new Array(len));
  };
  AudioContext.prototype.decodeAudioData = function (arrayBuffer) {
    // Hand the encoded bytes to the kernel decoder; returns an AudioBuffer.
    var ctx = this;
    var pcm = __webaudio_decode(arrayBuffer);
    return Promise.resolve(new AudioBuffer(ctx.sampleRate, pcm || []));
  };
  AudioContext.prototype.suspend = function () { this.state = 'suspended'; return Promise.resolve(); };
  AudioContext.prototype.resume = function () { this.state = 'running'; return Promise.resolve(); };
  AudioContext.prototype.close = function () { this.state = 'closed'; return Promise.resolve(); };

  g.AudioContext = AudioContext;
  g.webkitAudioContext = AudioContext;
  g.OfflineAudioContext = AudioContext;
})(self);
