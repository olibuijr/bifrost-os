// Drum Rack — 8 sample pads (track.instrument.pads = { note: sampleName }), Kit-compatible note numbers.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { getSample } = host
  const DEFAULT_CHOKES = { 42: 'hat', 44: 'hat', 46: 'hat' }
  const DEFAULT_PADS = { 36: 'kick.wav', 38: 'snare.wav', 39: 'clap.wav', 37: 'rim.wav', 42: 'hat.wav', 46: 'openhat.wav', 45: 'tom.wav', 50: 'perc.wav' }

  return {
    type: 'drumrack',
    name: 'Drum Rack',
    desc: 'Sample-based drum pads (instrument.pads maps note -> sample file). Notes like the Kit: 36 kick, 38 snare, 39 clap, 37 rim, 42 hat, 46 open hat, 45 tom, 50 perc. Closed hat chokes open hat (chokeMode 0 off, 1 hats, 2 hats+self; instrument.chokes {note:group} overrides)',
    sampled: 'pads',
    defaultPads: DEFAULT_PADS,
    pads: [[36, 'Kick'], [38, 'Snare'], [39, 'Clap'], [37, 'Rim'], [42, 'Hat'], [46, 'Open'], [45, 'Tom'], [50, 'Perc']],
    params: {
      tune:  [-24, 24, 0, 'int', 'st'],
      decay: [0.02, 4, 4, 'log', 's'],
      tone:  [200, 20000, 20000, 'log', 'Hz'],
      gain:  [0, 1, 0.85, 'lin', ''],
      chokeMode: [0, 2, 1, 'int', ''], // 0 off · 1 hat group (42/44 choke 46) · 2 hats + every pad chokes itself
    },
    create(ctx, out) {
      const groups = {} // choke group -> most recent vca
      return {
        trigger(p, pitch, vel, time, dur, inst) {
          const name = inst?.pads ? inst.pads[pitch] : DEFAULT_PADS[pitch]
          const buf = getSample(ctx, name); if (!buf) return
          const mode = p.chokeMode ?? 1
          const g = mode ? (inst?.chokes?.[pitch] ?? DEFAULT_CHOKES[pitch] ?? (mode === 2 ? 'self' + pitch : null)) : null
          const prev = g != null ? groups[g] : null
          if (prev) { prev.gain.cancelScheduledValues(time); prev.gain.setTargetAtTime(0, time, 0.006) }
          const rate = Math.pow(2, p.tune / 12)
          const src = ctx.createBufferSource(); src.buffer = buf; src.playbackRate.value = rate
          const vca = ctx.createGain(), f = ctx.createBiquadFilter()
          f.type = 'lowpass'; f.frequency.value = p.tone; f.Q.value = 0.5
          src.connect(f).connect(vca).connect(out)
          const peak = p.gain * vel, len = buf.duration / rate
          vca.gain.setValueAtTime(peak, time)
          let end = time + len
          if (p.decay < len) { vca.gain.setTargetAtTime(0, time + p.decay * 0.25, p.decay / 4); end = Math.min(end, time + p.decay * 1.6) }
          if (g != null) groups[g] = vca
          src.start(time); src.stop(end + 0.01)
          src.onended = () => { src.disconnect(); f.disconnect(); vca.disconnect(); if (g != null && groups[g] === vca) delete groups[g] }
        },
      }
    },
  }
}
