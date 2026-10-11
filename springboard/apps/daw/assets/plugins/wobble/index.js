// Wobble — mono bass with a tempo-synced LFO sweeping a 24 dB lowpass (dubstep / riddim / trap wobbles).
// Plugin: host API injected (no app imports).
const DIVS = [4, 2, 1, 0.5, 1 / 3, 0.25, 1 / 6, 0.125] // length in beats
const NH = 32
// band-limited LFO shape with a start phase (cycles), so 'Free' mode can lock to the song grid
function lfoWave(ctx, shape, phase) {
  const re = new Float32Array(NH + 1), im = new Float32Array(NH + 1), th = 2 * Math.PI * phase
  for (let n = 1; n <= NH; n++) {
    let b = 0
    if (shape === 0) b = n === 1 ? 1 : 0
    else if (shape === 1) b = n % 2 ? 8 / (Math.PI * Math.PI) * ((n - 1) / 2 % 2 ? -1 : 1) / (n * n) : 0
    else if (shape === 2) b = 2 / Math.PI / n // falling saw (starts mid, ramps down)
    else b = n % 2 ? 4 / Math.PI / n * 0.9 : 0
    re[n] = b * Math.sin(n * th); im[n] = b * Math.cos(n * th)
  }
  return ctx.createPeriodicWave(re, im, { disableNormalization: true })
}
export default function plugin(host) {
  // hardening: a present-but-non-finite param (bad automation/preset, NaN) falls back to its default,
  // finite values pass untouched (missing keys stay missing so old '??' fallbacks keep working)
  let P = null
  const sane = p => { let q = p; for (const k in P) { const x = p[k]; if (x !== undefined && (typeof x !== 'number' || !Number.isFinite(x))) { const y = +x; if (q === p) q = { ...p }; q[k] = Number.isFinite(y) ? y : P[k][2] } } return q }
  const { mtof } = host
  return {
    type: 'wobble',
    name: 'Wobble',
    desc: 'Mono dubstep/riddim/trap wobble bass: saw + square + sine sub through a 24 dB resonant lowpass swept by a tempo-synced LFO. Set rate in beat divisions (1/4, 1/8, 1/8T...); bpm 0 follows the song tempo. Change rate per clip via automation for talking wobbles',
    params: P = {
      rate:   [0, 7, 3, 'int', '', ['1/1', '1/2', '1/4', '1/8', '1/8T', '1/16', '1/16T', '1/32']],
      shape:  [0, 3, 0, 'int', '', ['Sine', 'Tri', 'Saw', 'Sqr']],
      bpm:    [0, 200, 0, 'lin', 'bpm'],
      lfoAmt: [0, 1, 0.7, 'lin', ''],
      retrig: [0, 1, 1, 'int', '', ['Free', 'Retrig']],
      octave: [-2, 1, -1, 'int', ''],
      cutoff: [40, 4000, 180, 'log', 'Hz'],
      reso:   [0, 20, 8, 'lin', ''],
      drive:  [0, 1, 0.5, 'lin', ''],
      sub:    [0, 1, 0.5, 'lin', ''],
      glide:  [0.005, 0.3, 0.04, 'log', 's'],
      gain:   [0, 1, 0.6, 'lin', ''],
    },
    create(ctx, out, env) {
      // pitch bend (optional engine hook bend(semis, at)): live pitched oscillators + new notes follow the bend
      let bendC = 0; const bent = new Set()
      const bn = o => { o._d0 = o.detune.value; o.detune.value = o._d0 + bendC; bent.add(o); o.addEventListener?.('ended', () => bent.delete(o)); return o }
      let cur = null, seq = 0
      const hold = (a, t) => a.cancelAndHoldAtTime ? a.cancelAndHoldAtTime(t) : a.cancelScheduledValues(t)
      const live = new Set()
      const shaper = ctx.createWaveShaper(), post = ctx.createGain()
      shaper.oversample = '2x'; shaper.connect(post).connect(out)
      let lastDrive = -1
      const setDrive = d => {
        if (d === lastDrive) return; const first = lastDrive < 0; lastDrive = d
        const k = 1 + d * 10, c = new Float32Array(1024)
        for (let i = 0; i < 1024; i++) { const x = i / 511.5 - 1; c[i] = Math.tanh(k * x) / Math.tanh(k) }
        shaper.curve = c; if (first) post.gain.value = 1 / (1 + d * 1.5); else post.gain.setTargetAtTime(1 / (1 + d * 1.5), ctx.currentTime, 0.01)
      }
      const songBpm = () => { try { const b = env?.engine?.proj?.bpm; return b > 20 ? b : 120 } catch { return 120 } }
      const lfoHz = p => (p.bpm >= 20 ? p.bpm : songBpm()) / 60 / DIVS[Math.max(0, Math.min(7, Math.round(p.rate)))]
      const oscs = v => [v.o1, v.o2, v.o3, v.lfo]
      const stopAll = (v, at) => { for (const o of oscs(v)) { try { o.stop(at) } catch {} } }
      const newLfo = (v, p, time) => {
        const lfo = ctx.createOscillator(), hz = lfoHz(p), sh = Math.max(0, Math.min(3, Math.round(p.shape || 0)))
        let ph = 0
        if (!p.retrig) { let t0 = 0; try { t0 = env?.engine?.startAt || 0 } catch {} ph = ((time - t0) * hz) % 1; if (!(ph >= 0)) ph = 0 }
        lfo.setPeriodicWave(lfoWave(ctx, sh, ph))
        lfo.frequency.setValueAtTime(hz, time)
        lfo.connect(v.depth); lfo.start(time); return lfo
      }
      const handle = (v, tok) => ({ release: at => {
        if (cur !== v || v.tok !== tok || at >= v.noteEnd) return
        const g = v.vca.gain; g.cancelAndHoldAtTime ? g.cancelAndHoldAtTime(at) : g.cancelScheduledValues(at)
        g.setTargetAtTime(0, at, 0.015); v.noteEnd = at; v.stopAt = at + 0.2; stopAll(v, v.stopAt)
      } })
      const setFilter = (v, p, time) => {
        const base = p.cutoff, depth = Math.min(ctx.sampleRate * 0.45 - base, base * p.lfoAmt * 12) / 2
        for (const f of [v.f1, v.f2]) { hold(f.frequency, time); f.frequency.setValueAtTime(base + depth, time) }
        hold(v.depth.gain, time); v.depth.gain.setTargetAtTime(depth, time, 0.005)
      }
      return {
        trigger(p, pitch, vel, time, dur) {
          if (!Number.isFinite(pitch) || !Number.isFinite(time)) return { release() {} }
          p = sane(p || {}); vel = Number.isFinite(vel) ? Math.max(0, Math.min(1, vel)) : 0.8; dur = Number.isFinite(dur) ? Math.max(0, dur) : 0.25
          setDrive(p.drive)
          const f = mtof(pitch + 12 * p.octave), len = Math.max(0.02, dur), off = time + len
          const peak = p.gain * 0.28 * (0.55 + 0.45 * vel)
          if (cur && cur.noteEnd >= time - 0.002 && cur.stopAt > time) { // legato glide
            const v = cur
            for (const [o, m] of [[v.o1, 1], [v.o2, 0.5], [v.o3, 0.5]]) { hold(o.frequency, time); o.frequency.setTargetAtTime(f * m, time, p.glide / 3) }
            v.subG.gain.setTargetAtTime(p.sub * 0.9, time, 0.01)
            setFilter(v, p, time); v.lfo.frequency.setTargetAtTime(lfoHz(p), time, 0.01)
            hold(v.vca.gain, time); v.vca.gain.setTargetAtTime(peak, time, 0.01)
            v.vca.gain.setTargetAtTime(0, off, 0.015)
            v.noteEnd = off; v.stopAt = off + 0.2; stopAll(v, v.stopAt)
            return handle(v, v.tok = ++seq)
          }
          const prev = cur && cur.stopAt > time ? cur : null
          if (prev) { hold(prev.vca.gain, time); prev.vca.gain.setTargetAtTime(0, time, 0.004); prev.stopAt = time + 0.04; stopAll(prev, prev.stopAt) }
          const o1 = ctx.createOscillator(), o2 = ctx.createOscillator(), o3 = ctx.createOscillator()
          const mixG = ctx.createGain(), sqG = ctx.createGain(), subG = ctx.createGain()
          const f1 = ctx.createBiquadFilter(), f2 = ctx.createBiquadFilter(), vca = ctx.createGain(), depth = ctx.createGain()
          o1.type = 'sawtooth'; o2.type = 'square'; o3.type = 'sine'; bn(o1); bn(o2); bn(o3)
          o1.frequency.setValueAtTime(f, time); o2.frequency.setValueAtTime(f / 2, time); o3.frequency.setValueAtTime(f / 2, time)
          sqG.gain.value = 0.6; subG.gain.value = p.sub * 0.9; mixG.gain.value = 0.7
          f1.type = f2.type = 'lowpass'; f1.Q.value = p.reso * 0.7; f2.Q.value = Math.min(p.reso * 0.35, 6)
          o1.connect(mixG); o2.connect(sqG).connect(mixG)
          mixG.connect(f1).connect(f2).connect(vca)
          o3.connect(subG).connect(vca) // sub bypasses the filter: solid low end under the wobble
          depth.connect(f1.frequency); depth.connect(f2.frequency)
          vca.connect(shaper)
          vca.gain.setValueAtTime(0, time); vca.gain.setTargetAtTime(peak, time, 0.002)
          vca.gain.setTargetAtTime(0, off, 0.015)
          const v = { o1, o2, o3, f1, f2, vca, depth, subG, noteEnd: off, stopAt: off + 0.2 }
          v.lfo = newLfo(v, p, time) // Retrig: phase 0 at note on; Free: phase locked to the song grid
          setFilter(v, p, time)
          o1.start(time); o2.start(time); o3.start(time)
          stopAll(v, v.stopAt)
          live.add(v)
          o1.onended = () => { vca.disconnect(); depth.disconnect(); live.delete(v); if (cur === v) cur = null }
          cur = v
          return handle(v, v.tok = ++seq)
        },
        bend(semis, at) { bendC = 100 * Math.max(-48, Math.min(48, +semis || 0)); const t = at ?? ctx.currentTime; for (const o of bent) { o.detune.cancelScheduledValues(t); o.detune.setTargetAtTime(o._d0 + bendC, t, 0.004) } },
        dispose() { bent.clear();
          const t = ctx.currentTime
          for (const v of live) { try { hold(v.vca.gain, t); v.vca.gain.setTargetAtTime(0, t, 0.005) } catch {} stopAll(v, t + 0.03) }
          live.clear(); cur = null
          setTimeout(() => { try { shaper.disconnect(); post.disconnect() } catch {} }, 60)
        },
      }
    },
  }
}
