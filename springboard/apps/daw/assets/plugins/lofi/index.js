// Lo-Fi — vinyl crackle bed + tape wow/flutter + band-limit aging + soft tanh saturation.
// Plugin: host API injected (no imports). Tempo-free.
export default function plugin(host) {
  const FILE = 'crackle.ogg', LOOP = 8.0, BASE = 0.008   // crackle loop length, base wow delay (s)
  const seen = new WeakSet()
  const ramp = (p, v) => { if (seen.has(p)) host.ramp(p, v); else { seen.add(p); p.value = v } }
  let curve = null
  const sat = () => {   // tanh over input domain ±4 so pre-gain can push into it without hard-clipping
    if (curve) return curve
    const n = 4096; curve = new Float32Array(n)
    for (let i = 0; i < n; i++) curve[i] = Math.tanh(((i / (n - 1)) * 2 - 1) * 4)
    return curve
  }
  return {
    type: 'lofi', name: 'Lo-Fi', desc: host.plugin?.desc,
    params: {
      crackle: [0, 1, 0.3, 'lin', ''],
      wow:     [0, 1, 0.3, 'lin', ''],
      age:     [0, 1, 0.4, 'lin', ''],
      drive:   [0, 1, 0.3, 'lin', ''],
      mix:     [0, 1, 1, 'lin', ''],
    },
    preload: ctx => host.loadBuffer(ctx, FILE),
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), dry = ctx.createGain(), wet = ctx.createGain()
      const pre = ctx.createGain(), sh = ctx.createWaveShaper(), post = ctx.createGain()
      const tape = ctx.createDelay(0.1), dryDelay = ctx.createDelay(0.1)
      const hp = ctx.createBiquadFilter(), lp = ctx.createBiquadFilter(), crackGain = ctx.createGain()
      sh.curve = sat(); sh.oversample = '2x'
      hp.type = 'highpass'; hp.Q.value = 0.6; lp.type = 'lowpass'; lp.Q.value = 0.5
      tape.delayTime.value = BASE; dryDelay.delayTime.value = BASE   // dry aligned with wet: no comb at partial mix
      input.connect(dryDelay).connect(dry).connect(output)
      input.connect(tape).connect(pre).connect(sh).connect(post).connect(hp).connect(lp).connect(wet).connect(output)
      crackGain.gain.value = 0; crackGain.connect(hp)   // crackle is band-limited with the music, scaled by mix
      // wow (slow drift) + flutter (fast) on the tape delay
      const wow = ctx.createOscillator(), flut = ctx.createOscillator(), wg = ctx.createGain(), fg = ctx.createGain()
      wow.frequency.value = 0.45; flut.frequency.value = 7.1
      wow.connect(wg).connect(tape.delayTime); flut.connect(fg).connect(tape.delayTime)
      wow.start(); flut.start()
      let src = null, dead = false
      const startLoop = buf => {
        if (dead || src || !buf) return
        src = ctx.createBufferSource(); src.buffer = buf; src.loop = true
        src.loopStart = 0; src.loopEnd = Math.min(LOOP, buf.duration)
        src.connect(crackGain); src.start(ctx.currentTime, Math.random() * LOOP * 0.9)
      }
      const buf = host.getBuffer(ctx, FILE)
      if (buf) startLoop(buf); else host.loadBuffer(ctx, FILE).then(startLoop, () => {})
      return {
        input, output,
        set(p) {
          const d = 1 + p.drive * 7
          ramp(pre.gain, d / 4); ramp(post.gain, 0.5 / Math.tanh(d * 0.5) * Math.pow(d, -0.3))  // ~unity at low drive, levels tamed when pushed
          ramp(hp.frequency, 20 + p.age * 130)                    // 20 → 150 Hz
          ramp(lp.frequency, 18000 * Math.pow(3000 / 18000, p.age)) // 18 k → 3 kHz
          ramp(wg.gain, p.wow * 0.0035); ramp(fg.gain, p.wow * 0.00035)
          ramp(crackGain.gain, p.crackle * p.crackle * 0.3)       // quiet bed: max ≈ -14 dBFS peaks
          ramp(wet.gain, p.mix); ramp(dry.gain, 1 - p.mix)
        },
        dispose() {
          dead = true
          for (const s of [wow, flut, src]) if (s) { try { s.stop() } catch {} s.disconnect() }
          src = null
        },
      }
    },
  }
}
