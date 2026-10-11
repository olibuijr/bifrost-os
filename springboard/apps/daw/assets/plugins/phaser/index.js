// Phaser — 5 allpass stages swept (in cents, i.e. log-frequency) by an LFO, with feedback.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'phaser', name: 'Phaser', desc: 'Swirling allpass phaser; rate = sweep speed, feedback = resonance',
    params: {
      rate:     [0.05, 8, 0.5, 'log', 'Hz'],
      depth:    [0, 1, 0.7, 'lin', ''],
      freq:     [200, 4000, 800, 'log', 'Hz'],
      feedback: [0, 0.9, 0.5, 'lin', ''],
      mix:      [0, 1, 0.5, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), dry = ctx.createGain(), wet = ctx.createGain(), fb = ctx.createGain()
      const lfo = ctx.createOscillator(), mod = ctx.createGain(); lfo.connect(mod); lfo.start()
      const stages = Array.from({ length: 5 }, () => { const a = ctx.createBiquadFilter(); a.type = 'allpass'; a.Q.value = 0.7; mod.connect(a.detune); return a })
      const sum = ctx.createGain(); input.connect(sum)
      stages.reduce((prev, a) => prev.connect(a), sum)
      const last = stages[stages.length - 1]
      last.connect(fb).connect(sum); last.connect(wet).connect(output); input.connect(dry).connect(output)
      // stop LFOs + detach so a removed/rebuilt effect doesn't keep running on the audio thread
      const dispose = () => { try { lfo.stop() } catch {} lfo.disconnect(); input.disconnect(); output.disconnect() }
      return { input, output, dispose, set(p) {
        ramp(lfo.frequency, p.rate); ramp(mod.gain, p.depth * 2400)
        stages.forEach((a, i) => ramp(a.frequency, p.freq * (1 + i * 0.35)))
        ramp(fb.gain, p.feedback); ramp(dry.gain, 1 - p.mix * 0.5); ramp(wet.gain, p.mix * 0.5)
      } }
    },
  }
}
