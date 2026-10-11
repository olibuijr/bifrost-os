// Crush — bitcrusher (quantizing WaveShaper) + sample-rate reduction approximation (resonant lowpass + fold).
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'crush', name: 'Crush', desc: 'Lo-fi bitcrusher/decimator; fewer bits and lower rate = dirtier',
    params: {
      bits: [1, 16, 6, 'int', 'bit'],
      rate: [500, 22000, 8000, 'log', 'Hz'],
      mix:  [0, 1, 1, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), dry = ctx.createGain(), wet = ctx.createGain()
      const q = ctx.createWaveShaper(), alias = ctx.createWaveShaper(), lp = ctx.createBiquadFilter()
      lp.type = 'lowpass'; lp.Q.value = 4
      input.connect(dry).connect(output)
      input.connect(alias).connect(q).connect(lp).connect(wet).connect(output)
      let lastBits = -1, lastRate = -1
      return { input, output, set(p) {
        if (p.bits !== lastBits) {
          lastBits = p.bits
          const n = 4096, c = new Float32Array(n), steps = Math.pow(2, p.bits - 1)
          for (let i = 0; i < n; i++) { const x = i / (n - 1) * 2 - 1; c[i] = Math.round(x * steps) / steps }
          q.curve = c
        }
        if (p.rate !== lastRate) {
          lastRate = p.rate
          // fake aliasing: add a small fold harmonic that grows as rate drops
          const amt = Math.max(0, 1 - p.rate / 22000) * 0.5, n = 2048, c = new Float32Array(n)
          for (let i = 0; i < n; i++) { const x = i / (n - 1) * 2 - 1; c[i] = (x + amt * Math.sin(x * Math.PI * 6) / 6) / (1 + amt / 6) }
          alias.curve = c
        }
        ramp(lp.frequency, p.rate / 2); ramp(dry.gain, 1 - p.mix); ramp(wet.gain, p.mix)
      } }
    },
  }
}
