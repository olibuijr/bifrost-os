// Haze — lush detuned supersaw / string pad with stereo spread and slow filter LFO.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'pad',
    name: 'Haze',
    desc: 'Lush supersaw/strings pad with wide stereo and slow filter motion. For sustained chords, ambient beds, trance/synthwave pads. Long attack & release by default — use long notes',
    params: P = {
      voices:  [3, 7, 7, 'int', ''],
      detune:  [0, 60, 22, 'lin', 'ct'],
      spread:  [0, 1, 0.8, 'lin', ''],
      octave:  [-2, 2, 0, 'int', ''],
      cutoff:  [100, 12000, 2200, 'log', 'Hz'],
      reso:    [0, 12, 1, 'lin', ''],
      lfoRate: [0.02, 4, 0.15, 'log', 'Hz'],
      lfoAmt:  [0, 1, 0.35, 'lin', ''],
      attack:  [0.005, 6, 0.9, 'log', 's'],
      release: [0.05, 8, 1.8, 'log', 's'],
      gain:    [0, 1, 0.6, 'lin', ''],
      decay:   [0.05, 8, 1, 'log', 's'],
      sustain: [0, 1, 1, 'lin', ''],
      fEnv:    [-1, 1, 0, 'lin', ''],
      fAtk:    [0.01, 8, 1.5, 'log', 's'],
      air:     [0, 1, 0, 'lin', ''],
      shape:   [0, 2, 0, 'int', '', ['Saw', 'Tri-Saw', 'Square']],
    },
    // Back-compat: sustain 1, fEnv 0, air 0, shape Saw == the original Haze.
    create(ctx, out, env) {
      // pitch bend (optional engine hook bend(semis, at)): live pitched oscillators + new notes follow the bend
      let bendC = 0; const bent = new Set()
      const bn = o => { o._d0 = o.detune.value; o.detune.value = o._d0 + bendC; bent.add(o); o.addEventListener?.('ended', () => bent.delete(o)); return o }
      const live = new Set(), nyq = ctx.sampleRate * 0.45
      const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
      // one free-running LFO per track, shared by every note: created with the instrument, stopped in dispose()
      let lfo = ctx.createOscillator(); lfo.frequency.value = 0.3; lfo.start()
      const getLfo = rate => {
        if (!lfo) { lfo = ctx.createOscillator(); lfo.frequency.value = rate; lfo.start() }
        else if (Math.abs(lfo.frequency.value - rate) > 1e-4) lfo.frequency.setTargetAtTime(rate, ctx.currentTime, 0.05)
        return lfo
      }
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          const f0 = mtof(pitch + 12 * p.octave)
          let n = Math.max(1, Math.round(p.voices)); if (live.size > 24) n = Math.ceil(n / 2)
          const flt = ctx.createBiquadFilter(), vca = ctx.createGain()
          flt.type = 'lowpass'; flt.Q.value = p.reso
          const lg = ctx.createGain(); lg.gain.value = p.cutoff * p.lfoAmt * 0.8
          const lf = p.lfoAmt > 0 ? getLfo(p.lfoRate) : null
          if (lf) lf.connect(lg).connect(flt.frequency)
          // 2-bus stereo: voices alternate onto a left or right bus panned by spread (2 panners per note)
          const bl = ctx.createStereoPanner(), br = ctx.createStereoPanner()
          bl.pan.value = -p.spread * 0.8; br.pan.value = p.spread * 0.8
          bl.connect(flt); br.connect(flt)
          const oscs = [], extra = [lg, bl, br, flt, vca]
          const sh = Math.round(p.shape || 0)
          for (let i = 0; i < n; i++) {
            const x = n > 1 ? i / (n - 1) * 2 - 1 : 0
            const o = ctx.createOscillator()
            o.type = sh === 2 ? 'square' : sh === 1 && i % 2 ? 'triangle' : 'sawtooth'; o.frequency.value = f0
            o.detune.value = x * p.detune + (Math.random() - 0.5) * 4; bn(o)
            o.connect(n === 1 ? flt : i % 2 ? br : bl); oscs.push(o)
          }
          let ns = null
          if (p.air > 0 && env && env.noise) {
            ns = ctx.createBufferSource(); const hp = ctx.createBiquadFilter(), ag = ctx.createGain()
            ns.buffer = env.noise; ns.loop = true; hp.type = 'highpass'; hp.frequency.value = 6000; ag.gain.value = p.air * 0.25
            ns.connect(hp).connect(ag).connect(vca); extra.push(hp, ag)
          }
          const peak = p.gain * (0.5 + 0.5 * vel) * 0.3 / Math.sqrt(n)
          const off = time + Math.max(0.05, dur)
          const aa = Math.max(0.005, p.attack), sus = p.sustain ?? 1
          vca.gain.setValueAtTime(0, time)
          vca.gain.setTargetAtTime(peak, time, aa / 3)
          if (sus < 1) vca.gain.setTargetAtTime(peak * sus, time + aa, p.decay / 3)
          vca.gain.cancelScheduledValues(off); vca.gain.setTargetAtTime(0, off, Math.max(0.0025, p.release / 4)) // short notes: drop a sustain target scheduled after note-off
          const fe = p.fEnv || 0
          if (fe) { // swell (+) opens from cutoff, sweep (-) closes toward it
            const lo = Math.max(40, Math.min(nyq, p.cutoff * 2 ** (-Math.abs(fe) * 4))), hi = Math.min(nyq, p.cutoff * 2 ** (Math.abs(fe) * 3))
            const a = fe > 0 ? lo : hi, b2 = p.cutoff
            flt.frequency.setValueAtTime(a, time); flt.frequency.setTargetAtTime(b2, time, p.fAtk / 3)
          } else flt.frequency.value = p.cutoff
          flt.connect(vca).connect(out)
          let end = off + p.release * 1.6 + 0.05
          if (ns) oscs.push(ns)
          for (const o of oscs) { o.start(time); o.stop(end) }
          const v = {}; live.add(v)
          oscs[0].onended = () => { for (const o of oscs) o.disconnect(); for (const e of extra) e.disconnect(); if (lf) { try { lf.disconnect(lg) } catch {} } live.delete(v) }
          v.kill = t => { try { hold(vca.gain, t); vca.gain.setTargetAtTime(0, t, 0.01); for (const o of oscs) o.stop(t + 0.05) } catch {} }
          return { release: at => { if (at >= end) return; hold(vca.gain, at); vca.gain.setTargetAtTime(0, at, p.release / 4); end = at + p.release * 1.6 + 0.05; for (const o of oscs) o.stop(end) } }
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const o of bent) { o.detune.cancelScheduledValues(t); o.detune.setTargetAtTime(o._d0 + bendC, t, 0.004) } },
        dispose() { bent.clear(); const t = ctx.currentTime; for (const v of live) v.kill(t); live.clear(); if (lfo) { try { lfo.stop(t + 0.1) } catch {} lfo = null } },
      }
    },
  }
}
