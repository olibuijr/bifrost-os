// Delay — tempo-synced feedback delay with damping.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'delay', name: 'Delay', desc: 'Tempo-synced delay, time in 16th steps',
    params: {
      time:     [1, 8, 3, 'int', '/16'],
      feedback: [0, 0.9, 0.35, 'lin', ''],
      tone:     [500, 16000, 4000, 'log', 'Hz'],
      mix:      [0, 1, 0.25, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain()
      const d = ctx.createDelay(4), fb = ctx.createGain(), lp = ctx.createBiquadFilter(), wet = ctx.createGain()
      input.connect(output); input.connect(d); d.connect(lp).connect(fb).connect(d); lp.connect(wet).connect(output)
      return { input, output, set(p, { bpm }) {
        ramp(d.delayTime, p.time * 60 / bpm / 4); ramp(fb.gain, p.feedback); ramp(lp.frequency, p.tone); ramp(wet.gain, p.mix)
      } }
    },
  }
}
