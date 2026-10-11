// Hyper — JP-8000 style supersaw with a small wavetable shape, stereo spread and a dedicated filter envelope.
// Plugin: host API injected (no app imports).
const JP = [-0.11002313, -0.06288439, -0.01952356, 0, 0.01991221, 0.06216538, 0.10745242]
const NH = 64
const waves = new WeakMap() // ctx -> [PeriodicWave x4]
function tables(ctx) {
  let w = waves.get(ctx); if (w) return w
  w = [0, 1, 2, 3].map(k => {
    const re = new Float32Array(NH + 1), im = new Float32Array(NH + 1)
    for (let n = 1; n <= NH; n++) {
      if (k === 0) im[n] = 1 / n // saw
      else if (k === 1) im[n] = (n % 2 ? 1 : 1.7) / n * (1 + 0.004 * n) // bright: even harmonics boosted
      else if (k === 2) re[n] = 2 / (n * Math.PI) * Math.sin(n * Math.PI * 0.25) * 1.6 // 25% pulse
      else im[n] = 1 / n * (0.5 + 2.4 * Math.exp(-(((n - 3.5) / 1.3) ** 2)) + 1.6 * Math.exp(-(((n - 8) / 2) ** 2)) + 0.6 * Math.exp(-(((n - 14) / 3) ** 2))) // vox formants
    }
    return ctx.createPeriodicWave(re, im)
  })
  waves.set(ctx, w); return w
}
// detune offsets (relative frequency) for n voices; exact JP table for 7
function offsets(n) {
  if (n === 7) return JP
  if (n === 1) return [0]
  const o = []
  for (let i = 0; i < n; i++) { const x = i / (n - 1) * 2 - 1; o.push(Math.sign(x) * 0.11 * Math.abs(x) ** 1.35) }
  if (n % 2 === 0) { let mi = 0; o.forEach((v, i) => { if (Math.abs(v) < Math.abs(o[mi])) mi = i }); o[mi] = 0 }
  return o
}
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'supersaw',
    name: 'Hyper',
    desc: 'JP-8000 style supersaw / mini-wavetable lead: up to 9 detuned voices with mix and stereo spread, shape (Saw, Bright, Hollow, Vox), lowpass with its own filter envelope, keytrack and delayed vibrato. Trance/EDM/future-bass leads, big chords and stabs',
    params: P = {
      voices:   [1, 9, 7, 'int', ''],
      detune:   [0, 1, 0.35, 'lin', ''],
      mix:      [0, 1, 0.6, 'lin', ''],
      spread:   [0, 1, 0.7, 'lin', ''],
      shape:    [0, 3, 0, 'int', '', ['Saw', 'Bright', 'Hollow', 'Vox']],
      octave:   [-2, 2, 0, 'int', ''],
      cutoff:   [60, 16000, 3000, 'log', 'Hz'],
      reso:     [0, 20, 1.5, 'lin', ''],
      keytrack: [0, 1, 0.5, 'lin', ''],
      vel2cut:  [0, 1, 0.4, 'lin', ''],
      fAmt:     [0, 1, 0.4, 'lin', ''],
      fAtk:     [0.001, 2, 0.005, 'log', 's'],
      fDec:     [0.01, 3, 0.4, 'log', 's'],
      fSus:     [0, 1, 0.3, 'lin', ''],
      attack:   [0.002, 2, 0.005, 'log', 's'],
      decay:    [0.01, 3, 0.5, 'log', 's'],
      sustain:  [0, 1, 0.8, 'lin', ''],
      release:  [0.01, 4, 0.3, 'log', 's'],
      vibrato:  [0, 1, 0, 'lin', ''],
      vibRate:  [2, 9, 5.5, 'lin', 'Hz'],
      vibDelay: [0, 2, 0.3, 'lin', 's'],
      glide:    [0, 0.5, 0, 'lin', 's'],
      velSens:  [0, 1, 0.6, 'lin', ''],
      gain:     [0, 1, 0.55, 'lin', ''],
    },
    create(ctx, out) {
      // pitch bend (optional engine hook bend(semis, at)): live pitched oscillators + new notes follow the bend
      let bendC = 0; const bent = new Set()
      const bn = o => { o._d0 = o.detune.value; o.detune.value = o._d0 + bendC; bent.add(o); o.addEventListener?.('ended', () => bent.delete(o)); return o }
      const live = new Set()
      let lastFreq = 0
      const W = tables(ctx), nyq = ctx.sampleRate * 0.45
      const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          let n = Math.max(1, Math.min(9, Math.round(p.voices)))
          if (n > 5 && pitch + 12 * p.octave > 84) n = 5 // CPU guard on high notes (aliasing-prone anyway)
          if (live.size > 24) n = Math.ceil(n / 2)
          const f0 = mtof(pitch + 12 * p.octave), wave = W[Math.max(0, Math.min(3, Math.round(p.shape)))]
          const amp = p.gain * (1 - p.velSens + p.velSens * vel ** 1.6)
          const cen = n > 1 ? 0.99785 - 0.55366 * p.mix : 1, side = n > 1 ? (-0.73764 * p.mix * p.mix + 1.2841 * p.mix + 0.044372) : 0
          const peak = amp * 0.3 / Math.sqrt(cen * cen + side * side * (n - 1))
          const det = p.detune ** 1.4, offs = offsets(n)
          const flt = ctx.createBiquadFilter(), vca = ctx.createGain(), cG = ctx.createGain()
          const busL = ctx.createGain(), busR = ctx.createGain()
          flt.type = 'lowpass'; flt.Q.value = p.reso
          cG.gain.value = cen; busL.gain.value = side; busR.gain.value = side
          cG.connect(flt)
          const nodes = [cG, busL, busR, flt, vca]
          if (n > 1) {
            const pl = ctx.createStereoPanner(), pr = ctx.createStereoPanner()
            pl.pan.value = -p.spread; pr.pan.value = p.spread
            busL.connect(pl).connect(flt); busR.connect(pr).connect(flt); nodes.push(pl, pr)
          }
          // delayed vibrato: one LFO per note shared by all voices
          let lfo = null, lg = null
          if (p.vibrato > 0) {
            lfo = ctx.createOscillator(); lg = ctx.createGain(); lfo.frequency.value = p.vibRate
            lg.gain.setValueAtTime(0, time); lg.gain.setTargetAtTime(p.vibrato * 40, time + p.vibDelay, 0.25)
            lfo.connect(lg); nodes.push(lg)
          }
          const oscs = []; let side_i = 0
          for (let i = 0; i < n; i++) {
            const o = ctx.createOscillator(), c = offs[i] === 0
            o.setPeriodicWave(wave)
            o.detune.value = 1200 * Math.log2(1 + offs[i] * det) + (c ? 0 : (Math.random() - 0.5) * 3); bn(o)
            if (p.glide > 0 && lastFreq) { o.frequency.setValueAtTime(lastFreq, time); o.frequency.exponentialRampToValueAtTime(f0, time + p.glide) }
            else o.frequency.setValueAtTime(f0, time)
            if (lg) lg.connect(o.detune)
            o.connect(c ? cG : (side_i++ % 2 ? busR : busL))
            o._t = c ? time : time + Math.random() * 0.002 // pseudo random phase
            oscs.push(o)
          }
          lastFreq = f0
          const off = time + Math.max(0.02, dur)
          const cut = Math.min(nyq, Math.max(30, p.cutoff * 2 ** (p.keytrack * (pitch + 12 * p.octave - 60) / 12) * (1 + p.vel2cut * (vel - 0.5))))
          const top = Math.min(nyq, cut * (1 + p.fAmt * 16)), sus = cut + (top - cut) * p.fSus
          const fa = Math.max(0.001, p.fAtk), aa = Math.max(0.002, p.attack)
          flt.frequency.setValueAtTime(cut, time)
          flt.frequency.linearRampToValueAtTime(top, time + fa)
          flt.frequency.setTargetAtTime(sus, time + fa, p.fDec / 3)
          flt.frequency.cancelScheduledValues(off); flt.frequency.setTargetAtTime(cut, off, p.release / 3)
          vca.gain.setValueAtTime(0, time)
          vca.gain.setTargetAtTime(peak, time, aa / 3)
          vca.gain.setTargetAtTime(peak * p.sustain, time + aa, p.decay / 3)
          vca.gain.cancelScheduledValues(off); vca.gain.setTargetAtTime(0, off, Math.max(0.0025, p.release / 4)) // short notes: drop a sustain target scheduled after note-off
          flt.connect(vca).connect(out)
          const v = { oscs, lfo }
          let end = off + p.release * 1.6 + 0.05
          for (const o of oscs) { o.start(o._t); o.stop(end) }
          if (lfo) { lfo.start(time); lfo.stop(end) }
          live.add(v)
          oscs[0].onended = () => { for (const x of nodes) x.disconnect(); for (const o of oscs) o.disconnect(); if (lfo) lfo.disconnect(); live.delete(v) }
          v.release = at => {
            if (at >= end) return
            hold(vca.gain, at); vca.gain.setTargetAtTime(0, at, p.release / 4)
            hold(flt.frequency, at); flt.frequency.setTargetAtTime(cut, at, p.release / 3)
            end = at + p.release * 1.6 + 0.05
            for (const o of oscs) o.stop(end); if (lfo) lfo.stop(end)
          }
          v.kill = t => { try { hold(vca.gain, t); vca.gain.setTargetAtTime(0, t, 0.005); for (const o of oscs) o.stop(t + 0.03); if (lfo) lfo.stop(t + 0.03) } catch {} }
          return { release: v.release }
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const o of bent) { o.detune.cancelScheduledValues(t); o.detune.setTargetAtTime(o._d0 + bendC, t, 0.004) } },
        dispose() { bent.clear(); const t = ctx.currentTime; for (const v of live) v.kill(t); live.clear() },
      }
    },
  }
}
