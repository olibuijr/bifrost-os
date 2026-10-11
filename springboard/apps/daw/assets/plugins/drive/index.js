// Drive — waveshaper saturation (tanh / soft-clip / tube) with tone control and auto gain compensation.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  const N = 2048, cache = new Map()
  // curve + its RMS gain for a -12 dBFS sine (autogain divides by it so perceived level stays put)
  function build(mode, amount) {
    const key = mode + ':' + Math.round(amount * 200); if (cache.has(key)) return cache.get(key)
    const k = 1 + amount * 60, c = new Float32Array(N)
    const f = mode === 1 ? x => { const u = Math.max(-1, Math.min(1, x * (1 + amount * 12))); return u - u * u * u / 3 } // cubic soft clip
      : mode === 2 ? x => { const u = x * (1 + amount * 20); return u >= 0 ? Math.tanh(u) : Math.tanh(u * 0.6) / 0.6 * 0.8 } // asymmetric tube
      : x => Math.tanh(k * x)
    let peak = 1e-9
    for (let i = 0; i < N; i++) { const x = i / ((N - 1) / 2) - 1; c[i] = f(x); peak = Math.max(peak, Math.abs(c[i])) }
    for (let i = 0; i < N; i++) c[i] /= peak
    const a = Math.pow(10, -12 / 20), S = 256; let si = 0, so = 0
    for (let j = 0; j < S; j++) {
      const x = a * Math.sin(2 * Math.PI * j / S), pos = (x + 1) * (N - 1) / 2, i0 = Math.floor(pos), fr = pos - i0
      const y = c[i0] + (c[Math.min(N - 1, i0 + 1)] - c[i0]) * fr, dc = (c[(N - 1) >> 1] + c[N >> 1]) / 2
      si += x * x; so += (y - dc) * (y - dc)
    }
    const r = { curve: c, gain: Math.sqrt(so / si) || 1 }
    cache.set(key, r); return r
  }
  return {
    type: 'drive', name: 'Drive', desc: 'Saturation / distortion (tanh, soft clip, tube) with auto gain',
    params: {
      amount:   [0, 1, 0.3, 'lin', ''],
      tone:     [500, 16000, 6000, 'log', 'Hz'],
      mix:      [0, 1, 1, 'lin', ''],
      mode:     [0, 2, 0, 'int', '', ['Tanh', 'Soft clip', 'Tube']],
      autogain: [0, 1, 1, 'int', '', ['Off', 'On']],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain()
      const sh = ctx.createWaveShaper(), dc = ctx.createBiquadFilter(), lp = ctx.createBiquadFilter(), wet = ctx.createGain(), dry = ctx.createGain()
      sh.oversample = '2x'; dc.type = 'highpass'; dc.frequency.value = 5; dc.Q.value = 0.707; let last = ''
      input.connect(sh).connect(dc).connect(lp).connect(wet).connect(output); input.connect(dry).connect(output)
      return { input, output, set(p) {
        const mode = Math.round(p.mode || 0), auto = (p.autogain ?? 1) >= 0.5, c = build(mode, p.amount)
        const k = mode + ':' + p.amount
        if (k !== last) { last = k; sh.curve = c.curve; ramp(dc.frequency, mode === 2 ? 20 : 5) }
        ramp(lp.frequency, p.tone)
        ramp(wet.gain, p.mix * (auto ? 1 / c.gain : 1 - p.amount * 0.4)); ramp(dry.gain, 1 - p.mix)
      } }
    },
  }
}
