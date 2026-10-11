// Reese — detuned twin/quad saws with moving phase, 24 dB lowpass, formant talk and a mono sub around the filter.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'reese',
    name: 'Reese',
    desc: "Mono reese bass for DnB, neuro and dark hip-hop: 2-4 detuned saws with slow phasing motion (beat Hz), 24 dB lowpass, drive, stereo width, formant 'talk' sweep (fmt) and a clean mono sine sub routed around the filter. Long sustained notes; overlapping notes glide",
    params: P = {
      saws:    [2, 4, 2, 'int', ''],
      detune:  [0, 40, 14, 'lin', 'ct'],
      beat:    [0.05, 2, 0.3, 'log', 'Hz'],
      motion:  [0, 1, 0.5, 'lin', ''],
      octave:  [-2, 1, -1, 'int', ''],
      cutoff:  [60, 6000, 700, 'log', 'Hz'],
      reso:    [0, 15, 2, 'lin', ''],
      drive:   [0, 1, 0.4, 'lin', ''],
      fmt:     [0, 1, 0, 'lin', ''],
      width:   [0, 1, 0.5, 'lin', ''],
      sub:     [0, 1, 0.6, 'lin', ''],
      attack:  [0.002, 1, 0.01, 'log', 's'],
      release: [0.01, 2, 0.15, 'log', 's'],
      glide:   [0.005, 0.4, 0.05, 'log', 's'],
      velSens: [0, 1, 0.4, 'lin', ''],
      gain:    [0, 1, 0.6, 'lin', ''],
    },
    create(ctx, out) {
      // pitch bend (optional engine hook bend(semis, at)): live pitched oscillators + new notes follow the bend
      let bendC = 0; const bent = new Set()
      const bn = o => { o._d0 = o.detune.value; o.detune.value = o._d0 + bendC; bent.add(o); o.addEventListener?.('ended', () => bent.delete(o)); return o }
      let cur = null, seq = 0
      const live = new Set()
      const shaper = ctx.createWaveShaper(), post = ctx.createGain()
      shaper.connect(post).connect(out)
      let lastDrive = -1
      const setDrive = d => {
        if (d === lastDrive) return; const first = lastDrive < 0; lastDrive = d
        const k = 1 + d * 7, c = new Float32Array(1024)
        for (let i = 0; i < 1024; i++) { const x = i / 511.5 - 1; c[i] = Math.tanh(k * x) / Math.tanh(k) }
        shaper.curve = c; if (first) post.gain.value = 1 / (1 + d * 1.2); else post.gain.setTargetAtTime(1 / (1 + d * 1.2), ctx.currentTime, 0.01)
      }
      const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
      const stopAll = (v, at) => { for (const o of v.oscs) { try { o.stop(at) } catch {} } }
      const env = (v, p, at) => { v.vca.gain.setTargetAtTime(0, at, p.release / 4); v.subV.gain.setTargetAtTime(0, at, p.release / 4) }
      const handle = (v, tok, p) => ({ release: at => {
        if (cur !== v || v.tok !== tok || at >= v.noteEnd) return
        hold(v.vca.gain, at); hold(v.subV.gain, at); env(v, p, at)
        v.noteEnd = at; v.stopAt = at + p.release * 1.6 + 0.05; stopAll(v, v.stopAt)
      } })
      const setFilters = (v, p, time) => {
        const nyq = ctx.sampleRate * 0.45, c = Math.min(nyq, p.cutoff)
        v.f1.frequency.setTargetAtTime(c, time, 0.01); v.f2.frequency.setTargetAtTime(c, time, 0.01)
        v.f1.Q.setTargetAtTime(p.reso * 0.7, time, 0.01); v.f2.Q.setTargetAtTime(Math.min(4, p.reso * 0.3), time, 0.01)
        v.pk.gain.setTargetAtTime(p.fmt * 15, time, 0.02); v.fmG.gain.setTargetAtTime(p.fmt * 650, time, 0.02)
        v.lfo.frequency.setTargetAtTime(p.beat, time, 0.02); v.mod.gain.setTargetAtTime(p.detune * p.motion, time, 0.02)
        v.pl.pan.setTargetAtTime(-p.width, time, 0.01); v.pr.pan.setTargetAtTime(p.width, time, 0.01)
      }
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          setDrive(p.drive)
          const f = mtof(pitch + 12 * p.octave), off = time + Math.max(0.02, dur)
          const amp = p.gain * (1 - p.velSens + p.velSens * vel ** 1.6)
          const n = Math.max(2, Math.min(4, Math.round(p.saws))), peak = amp * 0.3 / Math.sqrt(n), subPk = amp * 0.45 * p.sub
          const aa = Math.max(0.002, p.attack)
          if (cur && cur.noteEnd >= time - 0.002 && cur.stopAt > time) { // legato glide
            const v = cur
            for (const o of v.oscs) { if (o === v.lfo) continue; hold(o.frequency, time); o.frequency.setTargetAtTime(o === v.sub ? f / 2 : f, time, p.glide / 3) }
            setFilters(v, p, time)
            for (const [g, pk] of [[v.vca.gain, peak], [v.subV.gain, subPk]]) { hold(g, time); g.setTargetAtTime(pk, time, 0.01) }
            env(v, p, off); v.noteEnd = off; v.stopAt = off + p.release * 1.6 + 0.05; stopAll(v, v.stopAt)
            return handle(v, v.tok = ++seq, p)
          }
          if (cur && cur.stopAt > time) {
            const c = cur; hold(c.vca.gain, time); hold(c.subV.gain, time)
            c.vca.gain.setTargetAtTime(0, time, 0.004); c.subV.gain.setTargetAtTime(0, time, 0.004)
            c.stopAt = time + 0.04; stopAll(c, c.stopAt)
          }
          const pl = ctx.createStereoPanner(), pr = ctx.createStereoPanner(), f1 = ctx.createBiquadFilter(), f2 = ctx.createBiquadFilter()
          const pk = ctx.createBiquadFilter(), vca = ctx.createGain(), subV = ctx.createGain(), sub = bn(ctx.createOscillator())
          const lfo = ctx.createOscillator(), mod = ctx.createGain(), inv = ctx.createGain(), fmG = ctx.createGain()
          f1.type = f2.type = 'lowpass'; pk.type = 'peaking'; pk.Q.value = 4; pk.frequency.value = 900
          inv.gain.value = -1; lfo.connect(mod); mod.connect(inv); lfo.connect(fmG).connect(pk.frequency)
          const oscs = [], dets = [-0.5, 0.5, -1, 1]
          for (let i = 0; i < n; i++) {
            const o = ctx.createOscillator(); o.type = 'sawtooth'
            o.frequency.setValueAtTime(f, time); o.detune.value = dets[i] * p.detune; bn(o)
            if (i === 1) mod.connect(o.detune); if (i === 3) inv.connect(o.detune) // opposite motion = moving phase
            o.connect(i % 2 ? pr : pl); oscs.push(o)
          }
          pl.connect(f1); pr.connect(f1); f1.connect(f2).connect(pk).connect(vca).connect(shaper)
          sub.type = 'sine'; sub.frequency.setValueAtTime(f / 2, time); sub.connect(subV).connect(out) // mono, unfiltered
          vca.gain.setValueAtTime(0, time); vca.gain.setTargetAtTime(peak, time, aa / 3)
          subV.gain.setValueAtTime(0, time); subV.gain.setTargetAtTime(subPk, time, Math.max(0.003, aa / 3))
          oscs.push(sub, lfo)
          const v = { oscs, sub, lfo, mod, fmG, f1, f2, pk, pl, pr, vca, subV, noteEnd: off, stopAt: off + p.release * 1.6 + 0.05 }
          f1.frequency.value = f2.frequency.value = Math.min(ctx.sampleRate * 0.45, p.cutoff)
          setFilters(v, p, time); env(v, p, off)
          lfo.start(time); for (const o of oscs) if (o !== lfo) o.start(time)
          stopAll(v, v.stopAt); live.add(v)
          oscs[0].onended = () => { vca.disconnect(); subV.disconnect(); lfo.disconnect(); mod.disconnect(); inv.disconnect(); fmG.disconnect(); live.delete(v); if (cur === v) cur = null }
          cur = v
          return handle(v, v.tok = ++seq, p)
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const o of bent) { o.detune.cancelScheduledValues(t); o.detune.setTargetAtTime(o._d0 + bendC, t, 0.004) } },
        dispose() { bent.clear();
          const t = ctx.currentTime
          for (const v of live) { try { hold(v.vca.gain, t); hold(v.subV.gain, t); v.vca.gain.setTargetAtTime(0, t, 0.005); v.subV.gain.setTargetAtTime(0, t, 0.005) } catch {} stopAll(v, t + 0.03) }
          live.clear(); cur = null
          setTimeout(() => { try { shaper.disconnect(); post.disconnect() } catch {} }, 60)
        },
      }
    },
  }
}
