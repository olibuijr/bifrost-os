// Pulse — polyphonic subtractive synth.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'synth',
    name: 'Pulse',
    desc: 'Polyphonic subtractive synth (bass, leads, pads, keys)',
    params: P = {
      wave:    [0, 3, 2, 'int', '', ['Sine', 'Tri', 'Saw', 'Sqr']],
      voices:  [1, 7, 1, 'int', ''],
      detune:  [0, 50, 8, 'lin', 'ct'],
      width:   [0, 1, 0, 'lin', ''],
      octave:  [-2, 2, 0, 'int', ''],
      sub:     [0, 1, 0, 'lin', ''],
      osc2:    [0, 4, 0, 'int', '', ['Off', 'Sine', 'Tri', 'Saw', 'Sqr']],
      semi2:   [-24, 24, 0, 'int', 'st'],
      mix2:    [0, 1, 0.5, 'lin', ''],
      pwm:     [0, 1, 0, 'lin', ''],
      pwmRate: [0.1, 8, 0.8, 'log', 'Hz'],
      noise:   [0, 1, 0, 'lin', ''],
      cutoff:  [40, 16000, 2400, 'log', 'Hz'],
      reso:    [0, 20, 2, 'lin', ''],
      fType:   [0, 3, 0, 'int', '', ['LP12', 'LP24', 'BP', 'HP']],
      envAmt:  [0, 1, 0.4, 'lin', ''],
      legacyEnv: [0, 1, 1, 'int', '', ['Own', 'Amp']],
      fAtk:    [0.001, 2, 0.003, 'log', 's'],
      fDec:    [0.01, 3, 0.25, 'log', 's'],
      fSus:    [0, 1, 0.3, 'lin', ''],
      keyTrk:  [0, 1, 0, 'lin', ''],
      vel2cut: [0, 1, 0, 'lin', ''],
      attack:  [0.001, 2, 0.005, 'log', 's'],
      decay:   [0.01, 2, 0.25, 'log', 's'],
      sustain: [0, 1, 0.6, 'lin', ''],
      release: [0.01, 4, 0.2, 'log', 's'],
      glide:   [0, 0.5, 0, 'lin', 's'],
      gain:    [0, 1, 0.6, 'lin', ''],
    },
    // Back-compat: defaults reproduce the original Pulse (legacyEnv=1 -> filter env follows the amp env,
    // LP12, osc2 Off, pwm/noise/width/keyTrk/vel2cut 0).
    create(ctx, out, env) {
      // pitch bend (optional engine hook bend(semis, at)): live pitched oscillators + new notes follow the bend
      let bendC = 0; const bent = new Set()
      const bn = o => { o._d0 = o.detune.value; o.detune.value = o._d0 + bendC; bent.add(o); o.addEventListener?.('ended', () => bent.delete(o)); return o }
      let lastFreq = 0
      const live = new Set(), nyq = ctx.sampleRate * 0.45
      const WAVES = ['sine', 'triangle', 'sawtooth', 'square']
      const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
      const noiseBuf = env && env.noise
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          const f0 = mtof(pitch + 12 * p.octave)
          const vca = ctx.createGain(), flt = ctx.createBiquadFilter(), ft = Math.round(p.fType || 0)
          const mixIn = ctx.createGain(), nodes = [vca, flt, mixIn]
          let flt2 = null
          flt.type = ft === 2 ? 'bandpass' : ft === 3 ? 'highpass' : 'lowpass'
          if (ft === 1) { flt2 = ctx.createBiquadFilter(); flt2.type = 'lowpass'; flt.Q.value = flt2.Q.value = p.reso * 0.7; nodes.push(flt2) }
          else flt.Q.value = ft === 2 ? Math.max(0.5, p.reso) : p.reso
          mixIn.connect(flt); (flt2 ? flt.connect(flt2) : flt).connect(vca)
          const busy = live.size > 24
          let n = Math.max(1, Math.min(7, Math.round(p.voices)))
          if (busy) n = Math.ceil(n / 2)
          const use2 = p.osc2 > 0, usePwm = p.pwm > 0 && p.wave >= 2
          // CPU cap: <= 9 oscillators per note (a PWM voice costs 2)
          const per = usePwm ? 2 : 1, room = 9 - (use2 ? 1 : 0) - (p.sub > 0 && !busy ? 1 : 0)
          n = Math.max(1, Math.min(n, Math.floor(room / per)))
          const sub = p.sub > 0 && !busy ? p.sub : 0
          const peak = p.gain * vel * 0.32 / Math.sqrt(n + sub * 0.5 + (use2 ? p.mix2 : 0))
          const oscs = []
          const glideTo = o => { if (p.glide > 0 && lastFreq) { o.frequency.setValueAtTime(lastFreq * o._r, time); o.frequency.exponentialRampToValueAtTime(f0 * o._r, time + p.glide) } else o.frequency.setValueAtTime(f0 * o._r, time) }
          const mk = (type, r, det, dest) => { const o = ctx.createOscillator(); o.type = type; o._r = r; o.detune.value = det; bn(o); glideTo(o); o.connect(dest); oscs.push(o); return o }
          let pw = null
          if (usePwm) { // pulse = saw - delayed saw; delay (= width * period) swept by an LFO
            const lfo = ctx.createOscillator(), lg = ctx.createGain(); lfo.frequency.value = p.pwmRate
            const T = 1 / f0; lg.gain.value = T * 0.45 * p.pwm; lfo.connect(lg); oscs.push(lfo); nodes.push(lg); pw = { lg, T }
          }
          for (let i = 0; i < n; i++) {
            const det = n > 1 ? (i / (n - 1) - 0.5) * 2 * p.detune : 0
            let dest = mixIn
            if (p.width > 0 && n > 1) { const pan = ctx.createStereoPanner(); pan.pan.value = (i / (n - 1) * 2 - 1) * p.width; pan.connect(mixIn); nodes.push(pan); dest = pan }
            if (pw) {
              const g = ctx.createGain(), d = ctx.createDelay(0.1), inv = ctx.createGain()
              inv.gain.value = -1; d.delayTime.value = pw.T * 0.5; pw.lg.connect(d.delayTime)
              mk('sawtooth', 1, det, g); g.connect(dest); g.connect(d).connect(inv).connect(dest); nodes.push(g, d, inv)
            } else mk(WAVES[p.wave], 1, det, dest)
          }
          if (use2) { const g = ctx.createGain(); g.gain.value = p.mix2; g.connect(mixIn); nodes.push(g); mk(WAVES[p.osc2 - 1], 2 ** (p.semi2 / 12), 0, g) }
          if (sub) { const sg = ctx.createGain(); sg.gain.value = sub; sg.connect(mixIn); nodes.push(sg); mk('sine', 0.5, 0, sg) }
          let ns = null
          if (p.noise > 0 && noiseBuf) {
            ns = ctx.createBufferSource(); const ng = ctx.createGain(); ns.buffer = noiseBuf; ns.loop = true
            ng.gain.value = p.noise * 0.5; ns.connect(ng).connect(mixIn); nodes.push(ng)
          }
          lastFreq = f0
          const off = time + Math.max(0.02, dur)
          const legacy = p.legacyEnv !== 0
          const fa = legacy ? p.attack : p.fAtk, fd = legacy ? p.decay : p.fDec, fs = legacy ? p.sustain * 0.5 : p.fSus
          const cut = Math.min(nyq, Math.max(20, p.cutoff * 2 ** ((p.keyTrk || 0) * (pitch + 12 * p.octave - 60) / 12) * (1 + (p.vel2cut || 0) * (vel - 0.5))))
          const top = Math.min(18000, nyq, cut * (1 + p.envAmt * 12))
          const fparams = flt2 ? [flt.frequency, flt2.frequency] : [flt.frequency]
          for (const fp of fparams) {
            fp.setValueAtTime(cut, time)
            fp.linearRampToValueAtTime(top, time + fa)
            fp.setTargetAtTime(cut + (top - cut) * fs, time + fa, fd / 3)
            fp.setTargetAtTime(cut, off, p.release / 3)
          }
          const aa = Math.max(0.002, p.attack)
          vca.gain.setValueAtTime(0, time)
          if (legacy && p.attack > 0.004) vca.gain.linearRampToValueAtTime(peak, time + aa)
          else { vca.gain.setTargetAtTime(peak, time, aa / 3); vca.gain.setValueAtTime(peak, time + aa) } // anti-click start
          vca.gain.setTargetAtTime(peak * p.sustain, time + aa, p.decay / 3)
          vca.gain.cancelScheduledValues(off); vca.gain.setTargetAtTime(0, off, p.release / 4)
          vca.connect(out)
          let end = off + p.release * 1.6 + 0.05
          for (const o of oscs) { o.start(time); o.stop(end) }
          if (ns) { ns.start(time, Math.random() * Math.max(0, noiseBuf.duration - 0.1)); ns.stop(end); oscs.push(ns) }
          const v = {}
          live.add(v)
          oscs[0].onended = () => { for (const x of nodes) x.disconnect(); for (const o of oscs) o.disconnect(); live.delete(v) }
          v.release = at => { if (at >= end) return; hold(vca.gain, at); vca.gain.setTargetAtTime(0, at, p.release / 4); for (const fp of fparams) { hold(fp, at); fp.setTargetAtTime(cut, at, p.release / 3) } end = at + p.release * 1.6 + 0.05; for (const o of oscs) o.stop(end) }
          v.kill = t => { try { hold(vca.gain, t); vca.gain.setTargetAtTime(0, t, 0.005); for (const o of oscs) o.stop(t + 0.03) } catch {} }
          // held notes (engine.noteOn): release early
          return { release: v.release }
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const o of bent) { o.detune.cancelScheduledValues(t); o.detune.setTargetAtTime(o._d0 + bendC, t, 0.004) } },
        dispose() { bent.clear(); const t = ctx.currentTime; for (const v of live) v.kill(t); live.clear() },
      }
    },
  }
}
