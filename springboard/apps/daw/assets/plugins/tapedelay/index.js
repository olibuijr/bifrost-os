// Tape Delay — warm ping-pong echo with wow/flutter, saturation and darkening repeats.
// Plugin: host API injected (no imports).
export default function plugin(host) {
  // first write to a param jumps (no fade-in from node defaults); later writes glide via host.ramp
  const seen = new WeakSet()
  const ramp = (p, v) => { if (seen.has(p)) host.ramp(p, v); else { seen.add(p); p.value = v } }
  let satCurve = null
  function sat() {
    if (satCurve) return satCurve
    const n = 2048; satCurve = new Float32Array(n)
    for (let i = 0; i < n; i++) { const x = (i / (n - 1)) * 2 - 1; satCurve[i] = Math.tanh(x * 1.6) / Math.tanh(1.6) }
    return satCurve
  }
  return {
    type: 'tapedelay', name: 'Tape Delay',
    desc: 'Warm ping-pong tape echo, time in 16th steps, wow/flutter, saturated darkening repeats',
    params: {
      time:     [1, 16, 3, 'int', '/16'],
      feedback: [0, 0.95, 0.45, 'lin', ''],
      wow:      [0, 1, 0.3, 'lin', ''],
      tone:     [500, 12000, 3000, 'log', 'Hz'],
      mix:      [0, 1, 0.3, 'lin', ''],
      mode:     [0, 1, 1, 'int', '', ['Mono', 'Ping-pong']],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), dry = ctx.createGain(), wet = ctx.createGain()
      const sum = ctx.createGain(); sum.channelCount = 1; sum.channelCountMode = 'explicit'
      const dL = ctx.createDelay(5), dR = ctx.createDelay(5)
      const mkTone = () => {
        const hp = ctx.createBiquadFilter(), lp = ctx.createBiquadFilter(), sh = ctx.createWaveShaper(), fb = ctx.createGain()
        hp.type = 'highpass'; hp.frequency.value = 90; lp.type = 'lowpass'; lp.Q.value = 0.3; sh.curve = sat()
        hp.connect(lp).connect(sh).connect(fb); return { hp, lp, fb }
      }
      const tL = mkTone(), tR = mkTone()
      input.connect(dry).connect(output)
      input.connect(sum).connect(dL)
      dL.connect(tL.hp); tL.fb.connect(dR)   // L -> R hop
      dR.connect(tR.hp); tR.fb.connect(dL)   // R -> L hop
      // output routing: ping-pong (a=1,b=0) or mono (both taps both sides)
      const merge = ctx.createChannelMerger(2)
      const aL = ctx.createGain(), bL = ctx.createGain(), aR = ctx.createGain(), bR = ctx.createGain()
      tL.lp.connect(aL).connect(merge, 0, 0); tL.lp.connect(bL).connect(merge, 0, 1)
      tR.lp.connect(aR).connect(merge, 0, 1); tR.lp.connect(bR).connect(merge, 0, 0)
      merge.connect(wet).connect(output)
      // wow (slow) + flutter (fast) on both delay times
      const wow = ctx.createOscillator(), flut = ctx.createOscillator(), wg = ctx.createGain(), fg = ctx.createGain()
      wow.frequency.value = 0.55; flut.frequency.value = 6.3
      wow.connect(wg); flut.connect(fg)
      for (const g of [wg, fg]) { g.connect(dL.delayTime); g.connect(dR.delayTime) }
      wow.start(); flut.start()
      return {
        input, output,
        set(p, { bpm } = {}) {
          const t = Math.min(4.9, p.time * 60 / (bpm || 120) / 4)
          ramp(dL.delayTime, t); ramp(dR.delayTime, t)
          ramp(tL.fb.gain, p.feedback); ramp(tR.fb.gain, p.feedback)
          ramp(tL.lp.frequency, p.tone); ramp(tR.lp.frequency, p.tone)
          ramp(wg.gain, p.wow * 0.0025); ramp(fg.gain, p.wow * 0.0003)
          const pp = p.mode >= 0.5
          ramp(aL.gain, pp ? 1 : 0.7); ramp(aR.gain, pp ? 1 : 0.7); ramp(bL.gain, pp ? 0 : 0.7); ramp(bR.gain, pp ? 0 : 0.7)
          ramp(wet.gain, p.mix); ramp(dry.gain, 1 - p.mix * 0.3)
        },
        dispose() { try { wow.stop() } catch {} try { flut.stop() } catch {} wow.disconnect(); flut.disconnect() },
      }
    },
  }
}
