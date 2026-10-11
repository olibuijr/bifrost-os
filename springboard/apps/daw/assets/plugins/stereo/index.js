// Stereo — M/S width imager with bass-mono crossover. Plugin: host API injected (no imports).
export default function plugin(host) {
  // first write to a param jumps (no fade-in from node defaults); later writes glide via host.ramp
  const seen = new WeakSet()
  const ramp = (p, v) => { if (seen.has(p)) host.ramp(p, v); else { seen.add(p); p.value = v } }
  return {
    type: 'stereo', name: 'Stereo',
    desc: 'Stereo imager: M/S width (0 mono, 1 normal, 2 extra wide) with bass mono below freq',
    params: {
      width:    [0, 2, 1, 'lin', ''],
      monobass: [20, 400, 120, 'log', 'Hz'],
      bassmono: [0, 1, 1, 'int', '', ['Off', 'On']],
      pan:      [-1, 1, 0, 'lin', ''],
      gain:     [-12, 12, 0, 'lin', 'dB'],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain()
      const bq = (type) => { const f = ctx.createBiquadFilter(); f.type = type; f.Q.value = -3.01; return f }
      // Q is in dB for biquad LP/HP: -3.01 dB = Butterworth. Linkwitz-Riley 4th-order crossover (two cascaded Butterworth biquads per band)
      const lp1 = bq('lowpass'), lp2 = bq('lowpass'), hp1 = bq('highpass'), hp2 = bq('highpass')
      const mono = ctx.createGain(); mono.channelCount = 1; mono.channelCountMode = 'explicit'; mono.channelInterpretation = 'speakers'
      const lowOn = ctx.createGain()   // bass: mono'd or passed through
      const split = ctx.createChannelSplitter(2), merge = ctx.createChannelMerger(2)
      const ll = ctx.createGain(), rr = ctx.createGain(), lr = ctx.createGain(), rl = ctx.createGain()
      const hpOn = ctx.createGain(), full = ctx.createGain()   // crossover path vs. full-band path (bass mono off)
      const balL = ctx.createGain(), balR = ctx.createGain(), bsplit = ctx.createChannelSplitter(2), bmerge = ctx.createChannelMerger(2)
      const pre = ctx.createGain()
      // low band
      input.connect(lp1).connect(lp2).connect(mono).connect(lowOn).connect(pre)
      // high band -> width matrix
      input.connect(hp1).connect(hp2).connect(hpOn).connect(split)
      input.connect(full).connect(split)
      split.connect(ll, 0).connect(merge, 0, 0); split.connect(rl, 1).connect(merge, 0, 0)
      split.connect(rr, 1).connect(merge, 0, 1); split.connect(lr, 0).connect(merge, 0, 1)
      merge.connect(pre)
      pre.connect(bsplit); bsplit.connect(balL, 0).connect(bmerge, 0, 0); bsplit.connect(balR, 1).connect(bmerge, 0, 1)
      bmerge.connect(output)
      return { input, output, set(p) {
        const on = p.bassmono >= 0.5
        ramp(lowOn.gain, on ? 1 : 0); ramp(hpOn.gain, on ? 1 : 0); ramp(full.gain, on ? 0 : 1)
        for (const f of [lp1, lp2, hp1, hp2]) ramp(f.frequency, p.monobass)
        const w = p.width, a = (1 + w) / 2, b = (1 - w) / 2
        ramp(ll.gain, a); ramp(rr.gain, a); ramp(lr.gain, b); ramp(rl.gain, b)
        const g = Math.pow(10, p.gain / 20), th = (p.pan + 1) * Math.PI / 4
        ramp(balL.gain, g * Math.cos(th) * Math.SQRT2); ramp(balR.gain, g * Math.sin(th) * Math.SQRT2)
      } }
    },
  }
}
