// Limiter — mastering limiter: fast DynamicsCompressor + soft-knee clip at the ceiling.
// Plugin: host API injected (no imports).
export default function plugin(host) {
  // first write to a param jumps (no fade-in from node defaults); later writes glide via host.ramp
  const seen = new WeakSet()
  const ramp = (p, v) => { if (seen.has(p)) host.ramp(p, v); else { seen.add(p); p.value = v } }
  const curves = new Map(), HEAD = 1.12
  function clipCurve(knee) { // unity below knee, smooth saturation into ±1 (the ceiling)
    const k = Math.round(knee * 20) / 20; if (curves.has(k)) return curves.get(k)
    const n = 4096, c = new Float32Array(n), t = 1 - k
    for (let i = 0; i < n; i++) {
      const x = (i / (n - 1)) * 4 - 2, a = Math.abs(x)
      const y = a <= t ? a : t + (1 - t) * Math.tanh((a - t) / Math.max(1e-3, 1 - t))
      c[i] = Math.sign(x) * Math.min(1, y) / HEAD
    }
    curves.set(k, c); return c
  }
  return {
    type: 'limiter', name: 'Limiter',
    desc: 'Mastering brickwall limiter: input gain pushes loudness, output never exceeds ceiling',
    params: {
      gain:    [0, 24, 0, 'lin', 'dB'],
      ceiling: [-12, 0, -0.3, 'lin', 'dB'],
      release: [10, 1000, 120, 'log', 'ms'],
      soft:    [0, 1, 0.3, 'lin', ''],
      output:  [-12, 6, 0, 'lin', 'dB'],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain()
      const drive = ctx.createGain(), comp = ctx.createDynamicsCompressor(), unmake = ctx.createGain()
      const toUnit = ctx.createGain(), clip = ctx.createWaveShaper(), fromUnit = ctx.createGain(), trim = ctx.createGain()
      comp.ratio.value = 20; comp.attack.value = 0.0005; comp.knee.value = 0
      clip.oversample = '4x' // alias-free (Chrome's compressor already has ~6 ms internal lookahead); HEAD keeps the resampling-filter overshoot under the ceiling
      // WebAudio adds automatic makeup after the compressor; unmake cancels it so only `gain` sets loudness.
      // toUnit rescales so the ceiling maps to ±1 at the shaper (curve tops out at 1/HEAD)
      input.connect(drive).connect(comp).connect(unmake).connect(toUnit).connect(clip).connect(fromUnit).connect(trim).connect(output)
      let lastSoft = -1
      return { input, output, set(p) {
        const ceil = Math.pow(10, p.ceiling / 20)
        ramp(drive.gain, Math.pow(10, p.gain / 20))
        const th = p.ceiling - 1, knee = p.soft * 6
        comp.threshold.value = th; comp.knee.value = knee
        ramp(unmake.gain, Math.pow(10, th * (1 - 1 / 20) * 0.6 / 20)) // ≈ -(Chrome auto-makeup)
        ramp(comp.release, p.release / 1000)
        ramp(toUnit.gain, 1 / ceil); ramp(fromUnit.gain, ceil); ramp(trim.gain, Math.pow(10, p.output / 20))
        if (p.soft !== lastSoft) { lastSoft = p.soft; clip.curve = clipCurve(0.1 + p.soft * 0.4) }
      } }
    },
  }
}
