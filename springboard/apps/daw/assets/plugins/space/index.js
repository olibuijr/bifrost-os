// Space — convolution reverb with real measured impulse responses (OpenAIR, CC-BY 4.0).
// Plugin: host API injected (no imports). See README.md for IR attribution.
export default function plugin(host) {
  // first write to a param jumps (no fade-in from node defaults); later writes glide via host.ramp
  const seen = new WeakSet()
  const ramp = (p, v) => { if (seen.has(p)) host.ramp(p, v); else { seen.add(p); p.value = v } }
  const IRS = ['hall', 'cathedral', 'church', 'chamber', 'room', 'studio']
  const TRIM = [0.6, 1, 1, 1, 1, 1] // per-IR wet level match
  const LABELS = ['Hall', 'Cathedral', 'Church', 'Chamber', 'Room', 'Studio']
  // Per-context cache of decay-trimmed copies: ctx -> Map(key -> AudioBuffer)
  const trimmed = new WeakMap()

  function trim(ctx, src, decay) {
    let m = trimmed.get(ctx); if (!m) trimmed.set(ctx, m = new Map())
    const key = src.__id + '/' + decay
    let b = m.get(key); if (b) return b
    if (decay >= 1) { m.set(key, src); return src }
    const sr = src.sampleRate, len = Math.max(64, Math.floor(src.length * (0.25 + 0.75 * decay)))
    b = ctx.createBuffer(src.numberOfChannels, len, sr)
    const k = 5 * (1 / decay - 1) // extra exp decay per second
    for (let ch = 0; ch < src.numberOfChannels; ch++) {
      const s = src.getChannelData(ch), d = b.getChannelData(ch)
      for (let i = 0; i < len; i++) {
        const t = i / sr, fade = i > len * 0.8 ? (len - i) / (len * 0.2) : 1
        d[i] = s[i] * Math.exp(-k * t) * fade
      }
    }
    m.set(key, b); return b
  }

  return {
    type: 'space', name: 'Space',
    desc: 'Real-room convolution reverb (measured IRs: hall, cathedral, church, chamber, room, studio)',
    params: {
      space:    [0, 5, 0, 'int', '', LABELS],
      mix:      [0, 1, 0.25, 'lin', ''],
      predelay: [0, 200, 10, 'lin', 'ms'],
      decay:    [0.2, 1, 1, 'lin', 'x'],
      tone:     [1000, 18000, 9000, 'log', 'Hz'],
      width:    [0, 1.5, 1, 'lin', ''],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), dry = ctx.createGain()
      const pre = ctx.createDelay(0.5), cv = ctx.createConvolver(), lp = ctx.createBiquadFilter()
      const split = ctx.createChannelSplitter(2), merge = ctx.createChannelMerger(2)
      const ll = ctx.createGain(), rr = ctx.createGain(), lr = ctx.createGain(), rl = ctx.createGain()
      const wet = ctx.createGain(); wet.gain.value = 0
      lp.type = 'lowpass'; lp.Q.value = 0.5
      input.connect(dry).connect(output)
      input.connect(pre).connect(cv).connect(lp).connect(split)
      split.connect(ll, 0).connect(merge, 0, 0); split.connect(rl, 1).connect(merge, 0, 0)
      split.connect(rr, 1).connect(merge, 0, 1); split.connect(lr, 0).connect(merge, 0, 1)
      merge.connect(wet).connect(output)
      let want = '', loaded = '', mix = 0, token = 0, dead = false
      let trimG = 1
      const apply = () => ramp(wet.gain, loaded === want ? mix * 1.2 * trimG : 0)
      return {
        input, output,
        set(p) {
          const idx = Math.max(0, Math.min(IRS.length - 1, Math.round(p.space)))
          const dec = Math.round(Math.max(0.2, Math.min(1, p.decay)) * 20) / 20
          const key = idx + '/' + dec
          mix = p.mix
          ramp(dry.gain, 1 - p.mix * 0.5); ramp(pre.delayTime, p.predelay / 1000); ramp(lp.frequency, p.tone)
          const w = p.width, a = (1 + w) / 2, b = (1 - w) / 2
          ramp(ll.gain, a); ramp(rr.gain, a); ramp(lr.gain, b); ramp(rl.gain, b)
          if (key !== want) {
            want = key; const my = ++token
            host.loadBuffer(ctx, IRS[idx] + '.ogg').then(buf => {
              if (dead || my !== token) return
              if (!buf.__id) buf.__id = IRS[idx]
              cv.buffer = trim(ctx, buf, dec); trimG = TRIM[idx]; loaded = key; apply()
            }).catch(() => {})
          }
          apply() // dry-only until the IR is in
        },
        dispose() { dead = true; token++ },
      }
    },
  }
}
