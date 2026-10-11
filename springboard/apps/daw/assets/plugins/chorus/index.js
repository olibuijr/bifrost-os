// Chorus — stereo chorus: two delay lines modulated in anti-phase by one LFO, panned L/R.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'chorus', name: 'Chorus', desc: 'Stereo chorus; widens and thickens pads, guitars, vocals',
    params: {
      rate:  [0.05, 8, 0.8, 'log', 'Hz'],
      depth: [0, 1, 0.5, 'lin', ''],
      width: [0, 1, 0.8, 'lin', ''],
      mix:   [0, 1, 0.4, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), dry = ctx.createGain(), wet = ctx.createGain()
      const lfo = ctx.createOscillator(); lfo.start()
      const lines = [1, -1].map((sign, i) => {
        const d = ctx.createDelay(0.1), m = ctx.createGain(), pan = ctx.createStereoPanner()
        d.delayTime.value = i ? 0.019 : 0.014
        lfo.connect(m).connect(d.delayTime)
        input.connect(d).connect(pan).connect(wet)
        return { m, pan, sign }
      })
      input.connect(dry).connect(output); wet.connect(output)
      // stop LFOs + detach so a removed/rebuilt effect doesn't keep running on the audio thread
      const dispose = () => { try { lfo.stop() } catch {} lfo.disconnect(); input.disconnect(); output.disconnect() }
      return { input, output, dispose, set(p) {
        ramp(lfo.frequency, p.rate)
        for (const l of lines) { ramp(l.m.gain, l.sign * p.depth * 0.006); ramp(l.pan.pan, -l.sign * p.width) }
        ramp(dry.gain, 1 - p.mix * 0.5); ramp(wet.gain, p.mix * 0.7)
      } }
    },
  }
}
