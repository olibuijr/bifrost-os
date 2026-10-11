// EQ — low cut / low shelf / mid bell / high shelf / high cut. EQ3 params (low/mid/high) keep their meaning.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  // first write jumps (no sweep from node defaults on load); later writes glide
  const seen = new WeakSet()
  const ramp = (p, v) => { if (seen.has(p)) host.ramp(p, v); else { seen.add(p); p.value = v } }
  return {
    type: 'eq', name: 'EQ', desc: 'Parametric EQ: low cut, low shelf, mid bell, high shelf, high cut (gains in dB)',
    params: {
      low:      [-18, 18, 0, 'lin', 'dB'],
      mid:      [-18, 18, 0, 'lin', 'dB'],
      high:     [-18, 18, 0, 'lin', 'dB'],
      hpf:      [20, 500, 20, 'log', 'Hz'],
      lowFreq:  [60, 400, 220, 'log', 'Hz'],
      midFreq:  [200, 5000, 1100, 'log', 'Hz'],
      midQ:     [0.3, 4, 0.8, 'lin', ''],
      highFreq: [2000, 12000, 4500, 'log', 'Hz'],
      lpf:      [2000, 20000, 20000, 'log', 'Hz'],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain()
      const bq = type => { const f = ctx.createBiquadFilter(); f.type = type; return f }
      const hp = bq('highpass'), lo = bq('lowshelf'), mid = bq('peaking'), hi = bq('highshelf'), lp = bq('lowpass')
      hp.Q.value = 0.707; lp.Q.value = 0.707 // Butterworth-ish, no resonance bump
      input.connect(hp).connect(lo).connect(mid).connect(hi).connect(lp).connect(output)
      return { input, output, set(p) {
        // at the range ends the cut filters move out of the audio band (effectively bypassed)
        ramp(hp.frequency, (p.hpf ?? 20) <= 20.5 ? 5 : p.hpf)
        ramp(lp.frequency, (p.lpf ?? 20000) >= 19900 ? Math.min(ctx.sampleRate / 2, 24000) : p.lpf)
        ramp(lo.frequency, p.lowFreq ?? 220); ramp(mid.frequency, p.midFreq ?? 1100); ramp(mid.Q, p.midQ ?? 0.8); ramp(hi.frequency, p.highFreq ?? 4500)
        ramp(lo.gain, p.low); ramp(mid.gain, p.mid); ramp(hi.gain, p.high)
      } }
    },
  }
}
