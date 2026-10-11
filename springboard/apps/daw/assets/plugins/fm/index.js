// Operator — 2-operator FM synth with modulator feedback-ish drive.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'fm',
    name: 'Operator',
    desc: 'FM synth (carrier + modulator). Great for bells, glassy e-pianos (ratio 1, index ~2, fast idxDecay), punchy FM basses (ratio 1-2, high index, short decay), metallic tones (non-integer ratio)',
    params: P = {
      ratio:    [0.5, 12, 1, 'lin', ''],
      fine:     [-50, 50, 0, 'lin', 'ct'],
      index:    [0, 12, 3, 'lin', ''],
      idxDecay: [0.01, 4, 0.4, 'log', 's'],
      idxSus:   [0, 1, 0.2, 'lin', ''],
      feedback: [0, 1, 0, 'lin', ''],
      octave:   [-2, 2, 0, 'int', ''],
      attack:   [0.001, 2, 0.003, 'log', 's'],
      decay:    [0.01, 4, 0.8, 'log', 's'],
      sustain:  [0, 1, 0.3, 'lin', ''],
      release:  [0.01, 4, 0.4, 'log', 's'],
      gain:     [0, 1, 0.6, 'lin', ''],
      algo:     [0, 2, 0, 'int', '', ['2-Op', 'Stack 3', 'Parallel 2+tine']],
      ratio2:   [0.5, 16, 14, 'lin', ''],
      index2:   [0, 8, 0, 'lin', ''],
      idx2Decay:[0.005, 1, 0.08, 'log', 's'],
      velIdx:   [0, 1, 0.5, 'lin', ''],
      keyScale: [0, 1, 0, 'lin', ''],
    },
    // Back-compat: algo 0 + velIdx 0.5 + keyScale 0 == the original 2-op Operator.
    create(ctx, out) {
      // pitch bend (optional engine hook bend(semis, at)): live pitched oscillators + new notes follow the bend
      let bendC = 0; const bent = new Set()
      const bn = o => { o._d0 = o.detune.value; o.detune.value = o._d0 + bendC; bent.add(o); o.addEventListener?.('ended', () => bent.delete(o)); return o }
      const live = new Set()
      const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          const f0 = mtof(pitch + 12 * p.octave)
          const fm = f0 * p.ratio * Math.pow(2, p.fine / 1200)
          const ks = 2 ** (-(p.keyScale || 0) * (pitch + 12 * p.octave - 60) / 24) // higher notes decay faster
          const car = ctx.createOscillator(), mod = ctx.createOscillator(), idx = ctx.createGain(), vca = ctx.createGain()
          car.frequency.value = f0; mod.frequency.value = fm; bn(car); bn(mod)
          const oscs = [car, mod], nodes = [vca, idx]
          // "feedback": a second modulator at the same freq driving the modulator's frequency
          if (p.feedback > 0) {
            const fb = bn(ctx.createOscillator()), fg = ctx.createGain()
            fb.frequency.value = fm; fg.gain.value = p.feedback * fm * 1.5
            fb.connect(fg).connect(mod.frequency); oscs.push(fb); nodes.push(fg)
          }
          const off = time + Math.max(0.02, dur)
          const vi = p.velIdx ?? 0.5, depth = p.index * fm * (1 - vi + vi * vel)
          idx.gain.setValueAtTime(depth, time)
          idx.gain.setTargetAtTime(depth * p.idxSus, time, p.idxDecay * ks / 3)
          idx.gain.cancelScheduledValues(off); idx.gain.setTargetAtTime(0, off, p.release / 3)
          const peak = p.gain * vel * 0.3
          vca.gain.setValueAtTime(0, time)
          vca.gain.linearRampToValueAtTime(peak, time + Math.max(0.002, p.attack))
          vca.gain.setTargetAtTime(peak * p.sustain, time + Math.max(0.002, p.attack), p.decay * ks / 3)
          vca.gain.cancelScheduledValues(off); vca.gain.setTargetAtTime(0, off, p.release / 4)
          mod.connect(idx).connect(car.frequency)
          car.connect(vca).connect(out)
          const algo = Math.round(p.algo || 0)
          let idx2 = null
          if (algo > 0 && p.index2 > 0) { // third operator: fast-decaying high-ratio 'tine'
            const m2 = bn(ctx.createOscillator()), f2 = f0 * p.ratio2
            idx2 = ctx.createGain(); m2.frequency.value = f2; m2.connect(idx2); oscs.push(m2); nodes.push(idx2)
            const d2 = p.index2 * f2 * (1 - vi + vi * vel)
            idx2.gain.setValueAtTime(d2, time); idx2.gain.setTargetAtTime(0, time, p.idx2Decay * ks / 3)
            if (algo === 1) idx2.connect(mod.frequency) // stack: m2 -> mod -> car
            else { // parallel: m2 -> second carrier (ratio 1) mixed at 0.3
              const c2 = bn(ctx.createOscillator()), c2g = ctx.createGain(); c2.frequency.value = f0; c2g.gain.value = 0.3
              idx2.connect(c2.frequency); c2.connect(c2g).connect(vca); oscs.push(c2); nodes.push(c2g)
            }
          }
          let end = off + p.release * 1.6 + 0.05
          for (const o of oscs) { o.start(time); o.stop(end) }
          const v = {}; live.add(v)
          car.onended = () => { for (const x of nodes) x.disconnect(); for (const o of oscs) o.disconnect(); live.delete(v) }
          v.kill = t => { try { hold(vca.gain, t); vca.gain.setTargetAtTime(0, t, 0.005); for (const o of oscs) o.stop(t + 0.03) } catch {} }
          return { release: at => { if (at >= end) return; hold(vca.gain, at); vca.gain.setTargetAtTime(0, at, p.release / 4); hold(idx.gain, at); idx.gain.setTargetAtTime(0, at, p.release / 3); end = at + p.release * 1.6 + 0.05; for (const o of oscs) o.stop(end) } }
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const o of bent) { o.detune.cancelScheduledValues(t); o.detune.setTargetAtTime(o._d0 + bendC, t, 0.004) } },
        dispose() { bent.clear(); const t = ctx.currentTime; for (const v of live) v.kill(t); live.clear() },
      }
    },
  }
}
