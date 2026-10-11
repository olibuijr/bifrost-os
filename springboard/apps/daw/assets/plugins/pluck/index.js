// String — Karplus-Strong plucked string (DelayNode feedback loop + lowpass damping).
// A DelayNode inside a cycle has a minimum delay of one render quantum (128 samples),
// so high notes use a loop of N whole periods, excited by a strictly periodic burst
// (one noise period looped) — the loop then only carries harmonics of the true pitch.
// Plugin: host API injected (no app imports).
const excCache = new WeakMap() // ctx -> Map(key -> AudioBuffer), small LRU to avoid per-note allocation
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'pluck',
    name: 'String',
    desc: 'Karplus-Strong plucked string: guitar, harp, koto, plucked bass, pizzicato. Low damping + long decay = ringing harp; high damping = muted/palm-muted pluck',
    params: P = {
      damping:    [0, 1, 0.4, 'lin', ''],
      decay:      [0.1, 8, 1.5, 'log', 's'],
      brightness: [0, 1, 0.7, 'lin', ''],
      body:       [0, 1, 0.3, 'lin', ''],
      octave:     [-2, 2, 0, 'int', ''],
      gain:       [0, 1, 0.7, 'lin', ''],
      pick:       [0, 1, 0, 'lin', ''],
      tension:    [0, 1, 0, 'lin', ''],
      mute:       [0, 1, 0, 'lin', ''],
    },
    create(ctx, out, env) {
      // hardening: env.noise may be absent (offline/other hosts) -> make a private 1 s white-noise buffer
      let noise = env && env.noise
      if (!noise) { noise = ctx.createBuffer(1, ctx.sampleRate, ctx.sampleRate); const d = noise.getChannelData(0); for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1 }
      // pitch bend (optional engine hook bend(semis, at)): the string's loop delay follows the bend
      let bendC = 0
      const sr = ctx.sampleRate, minDelay = 128 / sr
      let cache = excCache.get(ctx); if (!cache) excCache.set(ctx, cache = new Map())
      const live = new Set()
      const excite = len => {
        const key = len * 4 + (Math.random() * 4 | 0) // 4 noise variants per length
        let buf = cache.get(key)
        if (buf) { cache.delete(key); cache.set(key, buf); return buf }
        buf = ctx.createBuffer(1, len, sr); const d = buf.getChannelData(0)
        const src0 = noise.getChannelData(0), off0 = Math.floor(Math.random() * (src0.length - len))
        let mean = 0
        for (let i = 0; i < len; i++) mean += (d[i] = src0[off0 + i])
        mean /= len; for (let i = 0; i < len; i++) d[i] -= mean
        cache.set(key, buf); if (cache.size > 64) cache.delete(cache.keys().next().value)
        return buf
      }
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          const f0 = mtof(pitch + 12 * p.octave), T = 1 / f0
          const periods = Math.max(1, Math.ceil((minDelay + 1 / sr) / T))
          const loopT = periods * T
          // excitation: one period of noise, looped for `periods` periods
          const len = Math.max(2, Math.round(T * sr))
          const buf = excite(len)
          const src = ctx.createBufferSource(); src.buffer = buf; src.loop = true
          const exc = ctx.createBiquadFilter(); exc.type = 'lowpass'
          exc.frequency.value = Math.min(18000, f0 * (1.5 + 30 * p.brightness * vel))
          const dl = ctx.createDelay(1), damp = ctx.createBiquadFilter(), fb = ctx.createGain()
          // lowpass adds a little group delay (~1/(2*pi*fc)); subtract a rough estimate
          const fc = Math.min(sr * 0.45, f0 * (1.2 + 20 * (1 - p.damping)))
          damp.type = 'lowpass'; damp.frequency.value = fc; damp.Q.value = -3 // WebAudio lowpass Q is resonance in dB: no peak inside the loop, or loop gain > 1 blows up
          dl.delayTime.value = Math.max(minDelay, loopT - 0.5 / (Math.PI * fc) * 0.5)
          const d0 = dl.delayTime.value; if (bendC) dl.delayTime.value = Math.max(minDelay, d0 * 2 ** (-bendC / 1200))
          // feedback gain so energy falls 60 dB in `decay` seconds
          const g = Math.min(0.995, Math.pow(0.001, loopT / p.decay)) // < 1 with margin: the loop must always decay
          fb.gain.value = g
          const vca = ctx.createGain(), bodyF = ctx.createBiquadFilter(), bodyG = ctx.createGain()
          bodyF.type = 'peaking'; bodyF.frequency.value = 180; bodyF.Q.value = 1.2; bodyF.gain.value = p.body * 9
          const peak = p.gain * vel * 0.3
          const extra = []
          if (p.pick > 0) { // pick position: feed-forward comb (exc - exc delayed by pick*T) notches harmonics
            const pd = ctx.createDelay(0.1), pinv = ctx.createGain()
            pd.delayTime.value = Math.min(0.09, Math.max(1 / sr, p.pick * 0.5 * T)); pinv.gain.value = -0.9
            exc.connect(pd).connect(pinv).connect(dl); extra.push(pd, pinv)
          }
          if (p.tension > 0) { // starts slightly sharp and settles (string tension on attack)
            const base = dl.delayTime.value
            dl.delayTime.setValueAtTime(Math.max(minDelay, base * 2 ** (-0.3 * p.tension / 12)), time)
            dl.delayTime.setTargetAtTime(base, time + 0.005, 0.05)
          }
          src.connect(exc).connect(dl); dl.connect(damp).connect(fb).connect(dl)
          dl.connect(bodyF).connect(vca).connect(out)
          const off = time + Math.max(dur, 0.05)
          const tail = Math.min(p.decay, 8)
          vca.gain.setValueAtTime(peak, time)
          vca.gain.setTargetAtTime(0, off, Math.max(0.03, Math.min(0.4, tail / 6)))
          src.start(time); src.stop(time + loopT)
          // keep-alive source to fire cleanup after the tail
          const end = Math.min(off + 1.2, time + tail) + 0.1
          const k = ctx.createConstantSource(); k.offset.value = 0; k.connect(bodyG); bodyG.gain.value = 0; bodyG.connect(vca)
          k.start(time); k.stop(end)
          vca.gain.setTargetAtTime(0, end - 0.08, 0.015)
          if (p.mute > 0) { // palm-release: damp the loop at note-off instead of only fading
            damp.frequency.setTargetAtTime(Math.max(40, fc + (f0 * 1.5 - fc) * p.mute), off, 0.01)
            fb.gain.setTargetAtTime(g + (Math.min(g, 0.9) - g) * p.mute, off, 0.01)
          }
          const v = { dl, d0 }; live.add(v)
          k.onended = () => { fb.disconnect(); dl.disconnect(); vca.disconnect(); bodyF.disconnect(); bodyG.disconnect(); for (const e of extra) e.disconnect(); live.delete(v) }
          const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
          v.kill = t => { try { hold(vca.gain, t); vca.gain.setTargetAtTime(0, t, 0.005); k.stop(t + 0.03) } catch {} }
          return { release: at => { if (at >= end) return; const tc = Math.max(0.03, Math.min(0.4, tail / 6)); hold(vca.gain, at); vca.gain.setTargetAtTime(0, at, tc); if (p.mute > 0) { hold(damp.frequency, at); damp.frequency.setTargetAtTime(Math.max(40, fc + (f0 * 1.5 - fc) * p.mute), at, 0.01); hold(fb.gain, at); fb.gain.setTargetAtTime(g + (Math.min(g, 0.9) - g) * p.mute, at, 0.01) } k.stop(Math.min(end, at + tc * 6 + 0.05)) } }
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const v of live) { v.dl.delayTime.cancelScheduledValues(t); v.dl.delayTime.setTargetAtTime(Math.max(minDelay, v.d0 * 2 ** (-bendC / 1200)), t, 0.004) } },
        dispose() { const t = ctx.currentTime; for (const v of live) v.kill(t); live.clear() },
      }
    },
  }
}
