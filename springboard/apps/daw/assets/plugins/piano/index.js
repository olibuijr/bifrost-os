// Grand — synthesized acoustic grand piano (native WebAudio only: PeriodicWave strings, noise hammer, decaying lowpass).
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  // ---- per-context caches ----
  const WAVES = new WeakMap()   // ctx -> Map(register -> PeriodicWave)
  const NOISE = new WeakMap()   // ctx -> AudioBuffer (1s white noise)

  // Register-dependent string spectrum: bass rich (many partials, slow rolloff), treble few partials.
  // Hammer strike position ~1/8 of string length notches every 8th partial (sin(pi*k/8) comb).
  function wave(ctx, pitch) {
    let m = WAVES.get(ctx); if (!m) WAVES.set(ctx, m = new Map())
    const reg = Math.max(0, Math.min(29, Math.round((pitch - 21) / 3)))
    let w = m.get(reg); if (w) return w
    const f0 = mtof(21 + reg * 3), nyq = Math.min(ctx.sampleRate / 2, 14000)
    const N = Math.max(3, Math.min(48, Math.floor(nyq / f0)))
    const x = (reg / 29), slope = 0.75 + x * 0.9           // rolloff steeper in treble
    const real = new Float32Array(N + 1), imag = new Float32Array(N + 1)
    for (let k = 1; k <= N; k++) {
      const comb = 0.25 + 0.75 * Math.abs(Math.sin(Math.PI * k / 8.3))
      // bass: fundamental weaker than 2nd/3rd partials (small soundboard radiation of low fundamentals)
      const fund = k === 1 && x < 0.3 ? 0.55 + x * 1.5 : 1
      imag[k] = fund * comb / Math.pow(k, slope) * (k % 2 ? 1 : 0.9)
    }
    w = ctx.createPeriodicWave(real, imag)
    m.set(reg, w); return w
  }
  function noise(ctx) {
    let b = NOISE.get(ctx); if (b) return b
    b = ctx.createBuffer(1, ctx.sampleRate >> 1, ctx.sampleRate)
    const d = b.getChannelData(0); let s = 12345
    for (let i = 0; i < d.length; i++) { s = (s * 1103515245 + 12345) & 0x7fffffff; d[i] = s / 0x3fffffff - 1 }
    NOISE.set(ctx, b); return b
  }
  const clamp = (v, a, b) => v < a ? a : v > b ? b : v
  // stretch tuning (Railsback-ish): bass flat, treble sharp, in cents
  const stretch = p => { const d = (p - 64) / 44; return Math.sign(d) * d * d * 22 }
  const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)

  return {
    type: 'piano',
    name: 'Grand',
    desc: 'Acoustic grand piano (synthesized). Velocity-sensitive tone and loudness, natural decay, damper release, stereo by pitch. For keys, ballads, chords, classical, jazz comping',
    params: P = {
      tone:     [0, 1, 0.55, 'lin', ''],
      hardness: [0, 1, 0.5, 'lin', ''],
      decay:    [0.2, 3, 1, 'log', 'x'],
      release:  [0.02, 1.5, 0.12, 'log', 's'],
      stereo:   [0, 1, 0.6, 'lin', ''],
      body:     [0, 1, 0.5, 'lin', ''],
      detune:   [0, 1, 0.35, 'lin', ''],
      hammer:   [0, 1, 0.4, 'lin', ''],
      velCurve: [0.5, 3, 1.6, 'lin', ''],
      gain:     [0, 1, 0.7, 'lin', ''],
    },
    create(ctx, out) {
      // shared soundboard body: low warmth + presence + slight top roll-off
      const b1 = ctx.createBiquadFilter(), b2 = ctx.createBiquadFilter(), b3 = ctx.createBiquadFilter()
      b1.type = 'peaking'; b1.frequency.value = 220; b1.Q.value = 1.1
      b2.type = 'peaking'; b2.frequency.value = 2600; b2.Q.value = 1.4
      b3.type = 'highshelf'; b3.frequency.value = 7000
      b1.connect(b2).connect(b3).connect(out)
      let lastBody = -1
      const live = new Set()
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          if (p.body !== lastBody) {
            const t0 = ctx.currentTime, first = lastBody < 0; lastBody = p.body // smoothed after the first note: no zipper on body automation
            if (first) { b1.gain.value = 4 * p.body; b2.gain.value = 2.5 * p.body; b3.gain.value = -3 * p.body } else {
            b1.gain.setTargetAtTime(4 * p.body, t0, 0.02); b2.gain.setTargetAtTime(2.5 * p.body, t0, 0.02); b3.gain.setTargetAtTime(-3 * p.body, t0, 0.02) }
          }
          const f0 = mtof(pitch), v = clamp(vel, 0, 1)
          const vc = Math.pow(v, p.velCurve)
          const reg = clamp((pitch - 21) / 87, 0, 1)            // 0 = A0, 1 = C8
          const oct = (pitch - 60) / 12

          // amplitude envelope times
          const tau1 = clamp(0.28 * p.decay * Math.pow(2, -oct * 0.55), 0.03, 2)        // prompt sound
          const tau2 = clamp(3.2 * p.decay * Math.pow(2, -oct * 0.75), 0.15, 14)       // aftersound
          const aft = 0.28 + 0.1 * (1 - reg)
          const peak = p.gain * (0.01 + 0.16 * vc) * (1.1 - 0.35 * reg)
          const undamped = pitch >= 89
          const relTau = Math.max(0.008, p.release / 4)
          const off = time + Math.max(0.03, dur)
          const nat = time + 0.004 + tau1 * 2.2 + tau2 * 6.5
          const end = undamped ? nat : Math.min(nat, off + relTau * 7)

          // strings
          const nStr = 2                                       // 2 detuned strings per note (CPU budget)
          const spread = p.detune * (0.6 + 2.4 * reg)          // cents, more beating in treble
          const baseDet = stretch(pitch), pw = wave(ctx, pitch)
          const lp = ctx.createBiquadFilter(), vca = ctx.createGain(), pan = ctx.createStereoPanner()
          const oscs = []
          for (let i = 0; i < nStr; i++) {
            const o = ctx.createOscillator()
            o.setPeriodicWave(pw); o.frequency.value = f0
            o.detune.value = baseDet + (nStr > 1 ? (i / (nStr - 1) - 0.5) * 2 * spread : 0)
            o.connect(lp); oscs.push(o)
          }
          // brightness: velocity x hardness x tone, decays as the note rings
          const hard = 0.25 + 0.75 * p.hardness, bright = 0.3 + 1.4 * p.tone
          const c0 = clamp(f0 * (1.6 + 26 * Math.pow(v, 1.4) * hard) * bright + 300 * v, 120, 18000)
          const c1 = clamp(f0 * (1.2 + 3 * v * hard) * bright + 150, 100, c0)
          lp.type = 'lowpass'; lp.Q.value = 0.6
          lp.frequency.setValueAtTime(c0, time)
          lp.frequency.setTargetAtTime(c1, time + 0.005, tau1 * 1.3 + 0.05)

          vca.gain.setValueAtTime(0, time)
          vca.gain.linearRampToValueAtTime(peak / nStr, time + 0.003)
          vca.gain.setTargetAtTime(peak / nStr * aft, time + 0.003, tau1)
          vca.gain.setTargetAtTime(0, time + 0.003 + tau1 * 2.2, tau2)
          if (!undamped && off < nat) { vca.gain.cancelScheduledValues(off); vca.gain.setTargetAtTime(0, off, relTau) }
          // (cancelScheduledValues at off drops only later events; the running target continues until the damper target)

          pan.pan.value = clamp((pitch - 64) / 40, -1, 1) * p.stereo
          lp.connect(vca).connect(pan).connect(b1)

          // hammer thump: short noise burst through the note's own (bright-at-onset) lowpass, <=40ms
          let ns = null, ng = null
          if (p.hammer > 0) {
            ns = ctx.createBufferSource(); ns.buffer = noise(ctx); ng = ctx.createGain()
            const nlen = clamp(0.012 - 0.006 * reg, 0.004, 0.012)
            const na = p.hammer * p.gain * 0.1 * Math.pow(v, 1.3) * (0.5 + 0.5 * hard)
            ng.gain.setValueAtTime(na, time); ng.gain.setTargetAtTime(0, time, nlen / 3)
            ns.connect(ng).connect(lp)
            ns.start(time, Math.random() * 0.3); ns.stop(time + 0.04)
            ns.onended = () => { ns.disconnect(); ng.disconnect() }
          }
          for (const o of oscs) { o.start(time); o.stop(end) }
          const vo = { kill: t => { try { hold(vca.gain, t); vca.gain.setTargetAtTime(0, t, 0.005); for (const o of oscs) o.stop(t + 0.03) } catch {} } }
          live.add(vo)
          oscs[0].onended = () => { for (const o of oscs) o.disconnect(); lp.disconnect(); vca.disconnect(); pan.disconnect(); live.delete(vo) }

          return {
            release: at => {
              if (undamped || at >= end) return
              hold(vca.gain, at); vca.gain.setTargetAtTime(0, at, relTau)
              const e = Math.max(at + relTau * 7, time + 0.01)
              for (const o of oscs) try { o.stop(e) } catch {}
            },
          }
        },
        dispose() { const t = ctx.currentTime; for (const v of live) v.kill(t); live.clear() },
      }
    },
  }
}
