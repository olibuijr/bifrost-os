// Sampler — pitched playback of one sample from the library (track.instrument.sample).
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { getSample, reverse: reverseOf } = host
  return {
    type: 'sampler',
    name: 'Sampler',
    desc: 'Plays one audio sample (instrument.sample) pitched across the keyboard relative to root. Vocal chops, stabs, textures, one-shots, loops',
    sampled: 'one',
    params: {
      root:    [24, 96, 60, 'int', ''],
      start:   [0, 1, 0, 'lin', ''],
      end:     [0, 1, 1, 'lin', ''],
      attack:  [0.001, 2, 0.002, 'log', 's'],
      release: [0.01, 4, 0.08, 'log', 's'],
      cutoff:  [60, 20000, 20000, 'log', 'Hz'],
      gain:    [0, 1, 0.8, 'lin', ''],
      mode:    [0, 1, 1, 'int', '', ['Gate', 'One-shot']],
      loop:    [0, 1, 0, 'int', '', ['Off', 'On']],
      reverse: [0, 1, 0, 'int', '', ['Off', 'On']],
    },
    create(ctx, out) {
      return {
        trigger(p, pitch, vel, time, dur, inst) {
          let buf = getSample(ctx, inst?.sample); if (!buf) return
          if (p.reverse) buf = reverseOf(ctx, buf)
          const len = buf.duration
          let a = Math.min(p.start, p.end), b = Math.max(p.start, p.end)
          if (b - a < 0.002) b = Math.min(1, a + 0.002)
          const off = a * len, regionLen = (b - a) * len
          const rate = Math.pow(2, (pitch - p.root) / 12)
          const src = ctx.createBufferSource(); src.buffer = buf; src.playbackRate.value = rate
          const vca = ctx.createGain(), f = ctx.createBiquadFilter()
          f.type = 'lowpass'; f.frequency.value = p.cutoff; f.Q.value = 0.7
          src.connect(f).connect(vca).connect(out)
          const peak = p.gain * vel, atk = Math.max(0.001, p.attack)
          vca.gain.setValueAtTime(0, time); vca.gain.linearRampToValueAtTime(peak, time + atk)
          const loop = !!p.loop, gate = loop || !p.mode
          let stop
          if (loop) { src.loop = true; src.loopStart = off; src.loopEnd = off + regionLen }
          const natural = time + regionLen / rate
          if (gate) {
            const noteOff = Math.max(time + atk, time + Math.max(dur, 0.02))
            const endAt = loop ? noteOff : Math.min(noteOff, natural)
            vca.gain.setValueAtTime(peak, endAt); vca.gain.setTargetAtTime(0, endAt, p.release / 4)
            stop = loop ? endAt + p.release * 1.5 : Math.min(natural, endAt + p.release * 1.5)
          } else stop = natural
          src.start(time, off, loop ? undefined : regionLen); src.stop(stop + 0.01)
          src.onended = () => { src.disconnect(); f.disconnect(); vca.disconnect() }
          // held notes (engine.noteOn): release early in gate mode; one-shots play out
          if (gate) return { release: at => {
            const g = vca.gain; g.cancelAndHoldAtTime ? g.cancelAndHoldAtTime(at) : g.cancelScheduledValues(at)
            g.setTargetAtTime(0, at, p.release / 4); try { src.stop(at + p.release * 1.5 + 0.01) } catch {}
          } }
        },
      }
    },
  }
}
