// Pump — tempo-synced sidechain-style ducking via a looping envelope buffer driving a gain.
// Phase is locked to the transport: ctx.startAt (ctx time of step 0) is passed by the engine on play.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { ramp } = host
  return {
    type: 'duck', name: 'Pump', desc: 'Sidechain pump: ducks volume on every beat division (rate in 16ths, 4 = quarter notes)',
    params: {
      amount:  [0, 1, 0.7, 'lin', ''],
      release: [0.05, 1, 0.5, 'lin', ''],
      curve:   [0.5, 4, 1.5, 'lin', ''],
      rate:    [1, 16, 4, 'int', '/16'],
      offset:  [-0.1, 0.1, 0, 'lin', 'beat'],
    },
    create(ctx) {
      const input = ctx.createGain(), output = ctx.createGain(), vca = ctx.createGain()
      vca.gain.value = 0; input.connect(vca).connect(output)
      const env = ctx.createGain(); env.connect(vca.gain)
      let src = null, key = '', buf = null, bufKey = ''
      const dispose = () => { try { src?.stop() } catch {} src?.disconnect(); input.disconnect(); output.disconnect() }
      return { input, output, dispose, set(p, { bpm, startAt } = {}) {
        bpm = bpm || 120
        const beat = 60 / bpm, period = p.rate * beat / 4, off = (p.offset || 0) * beat
        const bk = [period, p.release, p.curve].join(), k = bk + ',' + off + ',' + (startAt ?? '')
        // gain = (1-amount) + amount*env
        ramp(env.gain, p.amount); ramp(vca.gain, 1 - p.amount)
        if (k === key) return
        key = k
        if (bk !== bufKey) {
          bufKey = bk
          const n = Math.max(2, Math.round(period * ctx.sampleRate)); buf = ctx.createBuffer(1, n, ctx.sampleRate); const d = buf.getChannelData(0)
          const rel = Math.max(1, Math.round(n * p.release)), atk = Math.min(64, n >> 4)
          for (let i = 0; i < n; i++) d[i] = i < atk ? 1 - i / atk : i < rel ? Math.pow((i - atk) / (rel - atk), 1 / p.curve) : 1
        }
        if (src) { try { src.stop() } catch {} src.disconnect() }
        src = ctx.createBufferSource(); src.buffer = buf; src.loop = true
        src.connect(env)
        const now = ctx.currentTime + 0.01, origin = (Number.isFinite(startAt) ? startAt : 0) + off
        const ph = ((now - origin) % period + period) % period // position inside the loop at `now`
        src.start(now, ph)
      } }
    },
  }
}
