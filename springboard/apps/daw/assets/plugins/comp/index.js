// Comp — compressor with makeup gain and parallel mix.
// WebAudio's DynamicsCompressor adds automatic makeup; a trailing gain cancels it so `makeup` is the only makeup.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'comp', name: 'Comp', desc: 'Compressor (mix < 1 = parallel compression)',
    params: {
      threshold: [-60, 0, -18, 'lin', 'dB'],
      ratio:     [1, 20, 4, 'lin', ':1'],
      attack:    [0.001, 0.3, 0.01, 'log', 's'],
      release:   [0.02, 1, 0.2, 'log', 's'],
      makeup:    [0, 24, 3, 'lin', 'dB'],
      mix:       [0, 1, 1, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain()
      const c = ctx.createDynamicsCompressor(), mk = ctx.createGain(), dry = ctx.createGain()
      dry.gain.value = 0
      input.connect(c).connect(mk).connect(output); input.connect(dry).connect(output)
      return { input, output, set(p) {
        const mix = p.mix ?? 1, auto = -p.threshold * (1 - 1 / p.ratio) * 0.6 // ≈ Chrome auto-makeup (dB)
        ramp(c.threshold, p.threshold); ramp(c.ratio, p.ratio); ramp(c.attack, p.attack); ramp(c.release, p.release)
        ramp(mk.gain, Math.pow(10, (p.makeup - auto) / 20) * mix); ramp(dry.gain, 1 - mix)
      } }
    },
  }
}
