// Auto Filter — tempo-synced LFO sweeping a resonant lowpass. Plugin: host API injected (no imports).
export default function plugin(host) {
  // first write to a param jumps (no fade-in from node defaults); later writes glide via host.ramp
  const seen = new WeakSet()
  const ramp = (p, v) => { if (seen.has(p)) host.ramp(p, v); else { seen.add(p); p.value = v } }
  const SHAPES = ['sine', 'triangle', 'sawtooth', 'square']
  return {
    type: 'autofilter', name: 'Auto Filter',
    desc: 'Tempo-synced LFO filter sweep; rate = LFO period in 16th steps, depth in octaves',
    params: {
      rate:  [1, 32, 8, 'int', '/16'],
      depth: [0, 1, 0.6, 'lin', ''],
      freq:  [60, 8000, 700, 'log', 'Hz'],
      reso:  [0.5, 18, 5, 'log', 'Q'],
      shape: [0, 3, 0, 'int', '', ['Sine', 'Tri', 'Saw', 'Square']],
      mix:   [0, 1, 1, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), dry = ctx.createGain(), wet = ctx.createGain()
      const f1 = ctx.createBiquadFilter(), f2 = ctx.createBiquadFilter()
      f1.type = f2.type = 'lowpass'; f2.Q.value = 0.7
      const lfo = ctx.createOscillator(), amt = ctx.createGain()
      lfo.connect(amt); amt.connect(f1.detune); amt.connect(f2.detune)
      input.connect(dry).connect(output); input.connect(f1).connect(f2).connect(wet).connect(output)
      lfo.start()
      let shape = -1
      return {
        input, output,
        set(p, { bpm } = {}) {
          const s = Math.max(0, Math.min(3, Math.round(p.shape)))
          if (s !== shape) { shape = s; lfo.type = SHAPES[s] }
          ramp(lfo.frequency, 1 / (p.rate * 60 / (bpm || 120) / 4))
          ramp(amt.gain, p.depth * 2400)          // ±2 octaves at depth 1
          ramp(f1.frequency, p.freq); ramp(f2.frequency, p.freq)
          ramp(f1.Q, p.reso)
          ramp(wet.gain, p.mix * (1 / (1 + p.reso * 0.08))); ramp(dry.gain, 1 - p.mix)
        },
        dispose() { try { lfo.stop() } catch {} lfo.disconnect() },
      }
    },
  }
}
