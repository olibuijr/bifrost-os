// Mono — 303-style monophonic acid bass with accent and legato slide.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'bass',
    name: 'Mono',
    desc: 'Monophonic 303-style acid bass. High reso + envMod for squelch; velocity > 0.9 = accent (louder, sharper filter); overlapping/legato notes slide. Also fine for plain mono sub/saw basslines with reso low',
    params: P = {
      wave:    [0, 1, 0, 'int', '', ['Saw', 'Sqr']],
      octave:  [-2, 2, 0, 'int', ''],
      cutoff:  [40, 8000, 400, 'log', 'Hz'],
      reso:    [0, 30, 14, 'lin', ''],
      envMod:  [0, 1, 0.6, 'lin', ''],
      decay:   [0.03, 2, 0.25, 'log', 's'],
      accent:  [0, 1, 0.6, 'lin', ''],
      slide:   [0.01, 0.3, 0.06, 'log', 's'],
      drive:   [0, 1, 0.2, 'lin', ''],
      gain:    [0, 1, 0.6, 'lin', ''],
      lfoRate: [0, 7, 0, 'int', '', ['Off', '1/1', '1/2', '1/4', '1/8', '1/8T', '1/16', '1/32']],
      lfoBpm:  [0, 200, 0, 'lin', 'bpm'],
      lfoAmt:  [0, 1, 0, 'lin', ''],
      lfoDest: [0, 1, 0, 'int', '', ['Cutoff', 'Pitch']],
      sub:     [0, 1, 0, 'lin', ''],
    },
    create(ctx, out, env) {
      // pitch bend (optional engine hook bend(semis, at)): live pitched oscillators + new notes follow the bend
      let bendC = 0; const bent = new Set()
      const bn = o => { o._d0 = o.detune.value; o.detune.value = o._d0 + bendC; bent.add(o); o.addEventListener?.('ended', () => bent.delete(o)); return o }
      const DIV = [0, 4, 2, 1, 0.5, 1 / 3, 0.25, 0.125] // beats per LFO cycle (index 0 = Off)
      const songBpm = () => { try { const b = env?.engine?.proj?.bpm; return b > 20 ? b : 120 } catch { return 120 } }
      const lfoHz = p => (p.lfoBpm >= 20 ? p.lfoBpm : songBpm()) / 60 / DIV[p.lfoRate]
      const stopV = (v, at) => { v.osc.stop(at); if (v.sub) v.sub.stop(at); if (v.lfo) v.lfo.stop(at) }
      let cur = null, seq = 0 // { osc, flt, vca, end }
      const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
      const shaper = ctx.createWaveShaper(), post = ctx.createGain()
      let lastDrive = -1
      shaper.connect(post).connect(out)
      const setDrive = d => {
        if (d === lastDrive) return; const first = lastDrive < 0; lastDrive = d
        const k = 1 + d * 8, c = new Float32Array(1024)
        for (let i = 0; i < 1024; i++) { const x = i / 511.5 - 1; c[i] = Math.tanh(k * x) / Math.tanh(k) }
        shaper.curve = c; if (first) post.gain.value = 1 / (1 + d * 1.5); else post.gain.setTargetAtTime(1 / (1 + d * 1.5), ctx.currentTime, 0.01) // smoothed: no zipper on drive automation
      }
      // held notes (engine.noteOn): release early unless a newer note took over the mono voice
      const handle = (v, tok) => ({ release: at => {
        if (cur !== v || v.tok !== tok || at >= v.noteEnd) return
        const g = v.vca.gain; g.cancelAndHoldAtTime ? g.cancelAndHoldAtTime(at) : g.cancelScheduledValues(at)
        g.setTargetAtTime(0, at, 0.012); v.noteEnd = at; v.stopAt = at + 0.15; stopV(v, v.stopAt)
      } })
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          setDrive(p.drive)
          const f = mtof(pitch + 12 * p.octave), acc = vel > 0.9 ? p.accent : 0
          const len = Math.max(0.02, dur), off = time + len
          const peak = p.gain * 0.3 * (0.6 + 0.4 * vel) * (1 + acc * 0.5)
          const top = Math.min(16000, p.cutoff * (1 + p.envMod * 16 * (1 + acc)))
          const dec = p.decay * (1 - acc * 0.5)
          // legato: previous note still sounding at this start -> slide instead of retrigger
          if (cur && cur.noteEnd >= time - 0.002 && cur.stopAt > time) {
            const { osc, flt, vca } = cur
            hold(osc.frequency, time)
            osc.frequency.setTargetAtTime(f, time, p.slide / 3)
            if (cur.sub) { hold(cur.sub.frequency, time); cur.sub.frequency.setTargetAtTime(f / 2, time, p.slide / 3) }
            hold(vca.gain, time); vca.gain.setTargetAtTime(peak, time, 0.01)
            if (acc) { hold(flt.frequency, time); flt.frequency.setTargetAtTime(top, time, 0.005); flt.frequency.setTargetAtTime(p.cutoff, time + 0.02, dec / 3) }
            vca.gain.setTargetAtTime(0, off, 0.012)
            cur.noteEnd = off; cur.stopAt = off + 0.15
            stopV(cur, cur.stopAt)
            return handle(cur, cur.tok = ++seq)
          }
          if (cur && cur.stopAt > time) { // cut previous voice (mono)
            hold(cur.vca.gain, time); cur.vca.gain.setTargetAtTime(0, time, 0.003)
            stopV(cur, time + 0.03)
          }
          const osc = ctx.createOscillator(), flt = ctx.createBiquadFilter(), vca = ctx.createGain()
          osc.type = p.wave ? 'square' : 'sawtooth'; bn(osc); osc.frequency.setValueAtTime(f, time)
          flt.type = 'lowpass'; flt.Q.value = p.reso * (1 + acc * 0.3)
          flt.frequency.setValueAtTime(top, time)
          flt.frequency.setTargetAtTime(p.cutoff, time + 0.003, dec / 3)
          vca.gain.setValueAtTime(0, time)
          vca.gain.linearRampToValueAtTime(peak, time + 0.003)
          vca.gain.setTargetAtTime(peak * 0.7, time + 0.003, dec)
          vca.gain.setTargetAtTime(0, off, 0.012)
          osc.connect(flt).connect(vca).connect(shaper)
          const v = { osc, flt, vca, noteEnd: off, stopAt: off + 0.15 }
          const rate = Math.round(p.lfoRate || 0)
          if (rate > 0 && rate < 8 && p.lfoAmt > 0) { // tempo-synced wobble, restarted per (non-slide) note
            const lfo = ctx.createOscillator(), lg = ctx.createGain()
            lfo.frequency.value = lfoHz({ ...p, lfoRate: rate })
            if (p.lfoDest) { lg.gain.value = p.lfoAmt * 100; lfo.connect(lg).connect(osc.detune) }
            else { lg.gain.value = Math.min(ctx.sampleRate * 0.45 - top, p.cutoff * p.lfoAmt * 10) / 2; lfo.connect(lg).connect(flt.frequency) }
            lfo.start(time); v.lfo = lfo; v.lg = lg
          }
          if (p.sub > 0) {
            const sub = bn(ctx.createOscillator()), sg = ctx.createGain()
            sub.frequency.setValueAtTime(f / 2, time); sg.gain.value = p.sub * 0.8
            sub.connect(sg).connect(vca); sub.start(time); v.sub = sub; v.sg = sg
          }
          osc.start(time); stopV(v, v.stopAt)
          osc.onended = () => { vca.disconnect(); if (v.lg) { v.lfo.disconnect(); v.lg.disconnect() } if (v.sg) v.sg.disconnect(); if (cur === v) cur = null }
          cur = v
          return handle(v, v.tok = ++seq)
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const o of bent) { o.detune.cancelScheduledValues(t); o.detune.setTargetAtTime(o._d0 + bendC, t, 0.004) } },
        dispose() { bent.clear();
          const t = ctx.currentTime
          if (cur) { try { cur.vca.gain.cancelScheduledValues(t); cur.vca.gain.setTargetAtTime(0, t, 0.005); stopV(cur, t + 0.03) } catch {} }
          cur = null; setTimeout(() => { try { shaper.disconnect(); post.disconnect() } catch {} }, 60)
        },
      }
    },
  }
}
