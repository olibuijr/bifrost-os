// Tremolo — tempo-synced amplitude or auto-pan modulation.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'tremolo', name: 'Tremolo', desc: 'Tempo-synced tremolo/autopan; rate = LFO period in 16th steps',
    params: {
      rate:  [1, 16, 2, 'int', '/16'],
      depth: [0, 1, 0.6, 'lin', ''],
      shape: [0, 2, 0, 'int', '', ['Sine', 'Square', 'Tri']],
      pan:   [0, 1, 0, 'int', '', ['Trem', 'Pan']],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), vca = ctx.createGain(), panner = ctx.createStereoPanner()
      const lfo = ctx.createOscillator(), amp = ctx.createGain(), pm = ctx.createGain()
      lfo.connect(amp).connect(vca.gain); lfo.connect(pm).connect(panner.pan); lfo.start()
      input.connect(vca).connect(panner).connect(output)
      let lastShape = -1
      // stop LFOs + detach so a removed/rebuilt effect doesn't keep running on the audio thread
      const dispose = () => { try { lfo.stop() } catch {} lfo.disconnect(); input.disconnect(); output.disconnect() }
      return { input, output, dispose, set(p, { bpm }) {
        if (p.shape !== lastShape) { lastShape = p.shape; lfo.type = ['sine', 'square', 'triangle'][p.shape] || 'sine' }
        ramp(lfo.frequency, 1 / (p.rate * 60 / bpm / 4))
        const trem = p.pan ? 0 : p.depth / 2
        ramp(vca.gain, 1 - trem); ramp(amp.gain, trem); ramp(pm.gain, p.pan ? p.depth : 0)
      } }
    },
  }
}
