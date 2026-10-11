// Filter — resonant LP/HP/BP.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'filter', name: 'Filter', desc: 'Resonant filter',
    params: {
      type: [0, 2, 0, 'int', '', ['LP', 'HP', 'BP']],
      freq: [40, 18000, 18000, 'log', 'Hz'],
      q:    [0, 20, 1, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), f = ctx.createBiquadFilter()
      input.connect(f).connect(output)
      return { input, output, set(p) { f.type = ['lowpass', 'highpass', 'bandpass'][p.type] || 'lowpass'; ramp(f.frequency, p.freq); ramp(f.Q, p.q) } }
    },
  }
}
