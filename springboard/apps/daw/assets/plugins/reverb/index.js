// Reverb — convolution with a generated, damped noise impulse.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  function impulse(ctx, secs, damp) {
    const sr = ctx.sampleRate, len = Math.floor(sr * secs), buf = ctx.createBuffer(2, len, sr)
    const a = Math.exp(-2 * Math.PI * damp / sr)
    for (let ch = 0; ch < 2; ch++) {
      const d = buf.getChannelData(ch); let lp = 0
      for (let i = 0; i < len; i++) {
        const n = (Math.random() * 2 - 1) * Math.pow(1 - i / len, 3)
        lp = lp * a + n * (1 - a); d[i] = lp * (1 + damp / 8000)
      }
    }
    return buf
  }

  return {
    type: 'reverb', name: 'Reverb', desc: 'Convolution reverb, size = tail seconds',
    params: {
      size: [0.2, 6, 2, 'lin', 's'],
      damp: [1000, 16000, 6000, 'log', 'Hz'],
      mix:  [0, 1, 0.25, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain()
      const cv = ctx.createConvolver(), wet = ctx.createGain(), dry = ctx.createGain(); let key = ''
      input.connect(dry).connect(output); input.connect(cv).connect(wet).connect(output)
      return { input, output, set(p) {
        const k = p.size.toFixed(1) + '/' + Math.round(p.damp / 500)
        if (k !== key) { key = k; cv.buffer = impulse(ctx, p.size, p.damp) }
        ramp(wet.gain, p.mix * 1.4); ramp(dry.gain, 1 - p.mix * 0.5)
      } }
    },
  }
}
