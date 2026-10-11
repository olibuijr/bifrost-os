// Kit — synthesized drum machine (808/909-ish). One voice per GM-style note.
// Plugin: host API injected (no app imports).
export default function plugin(host) {
  const { mtof } = host
  return {
    type: 'drums',
    name: 'Kit',
    desc: 'Synthesized drum machine. Notes: 36 kick, 37 rim, 38 snare, 39 clap, 42 closed hat, 46 open hat, 45 low tom, 50 high tom',
    pads: [[36, 'Kick'], [38, 'Snare'], [39, 'Clap'], [37, 'Rim'], [42, 'Hat'], [46, 'Open'], [45, 'Tom L'], [50, 'Tom H']],
    params: {
      kickTune:  [30, 90, 48, 'lin', 'Hz'],
      kickDecay: [0.1, 1.2, 0.45, 'lin', 's'],
      kickPunch: [0, 1, 0.6, 'lin', ''],
      snareTone: [120, 320, 190, 'lin', 'Hz'],
      snareSnap: [0, 1, 0.6, 'lin', ''],
      hatTone:   [4000, 12000, 8000, 'log', 'Hz'],
      hatDecay:  [0.02, 0.5, 0.06, 'log', 's'],
      tomTune:   [60, 200, 110, 'lin', 'Hz'],
      gain:      [0, 1, 0.8, 'lin', ''],
    },
    create(ctx, dest, { noise }) {
      let openHat = null
      return {
        trigger(p, pitch, vel, time) {
          const out = ctx.createGain()
          out.gain.value = p.gain * vel; out.connect(dest)
          const env = (node, a, d) => { const e = ctx.createGain(); e.gain.setValueAtTime(a, time); e.gain.exponentialRampToValueAtTime(0.0008, time + d); node.connect(e).connect(out); return e }
          const osc = (type, f) => { const o = ctx.createOscillator(); o.type = type; o.frequency.value = f; return o }
          const nz = () => { const s = ctx.createBufferSource(); s.buffer = noise; s.loop = true; s.loopStart = Math.random() * 0.5; return s }
          const filt = (type, f, q = 0.7) => { const b = ctx.createBiquadFilter(); b.type = type; b.frequency.value = f; b.Q.value = q; return b }
          const play = (src, len) => { src.start(time); src.stop(time + len + 0.05) }
          switch (pitch) {
            case 36: { // kick
              const o = osc('sine', p.kickTune)
              o.frequency.setValueAtTime(p.kickTune * (3 + p.kickPunch * 5), time)
              o.frequency.exponentialRampToValueAtTime(p.kickTune, time + 0.03 + (1 - p.kickPunch) * 0.05)
              env(o, 1, p.kickDecay); play(o, p.kickDecay)
              const n = nz(); env(n.connect(filt('highpass', 3000)), p.kickPunch * 0.25, 0.012); play(n, 0.02)
              break
            }
            case 38: case 40: { // snare
              const o = osc('triangle', p.snareTone); o.frequency.exponentialRampToValueAtTime(p.snareTone * 0.7, time + 0.08)
              env(o, 0.7, 0.14); play(o, 0.15)
              const n = nz(); env(n.connect(filt('highpass', 1400)), 0.4 + p.snareSnap * 0.5, 0.12 + p.snareSnap * 0.12); play(n, 0.26)
              break
            }
            case 39: { // clap
              const n = nz(), e = ctx.createGain()
              n.connect(filt('bandpass', 1300, 1.2)).connect(e).connect(out)
              for (const [dt, a] of [[0, 0.9], [0.011, 0.8], [0.022, 0.9]]) { e.gain.setValueAtTime(a, time + dt); e.gain.exponentialRampToValueAtTime(0.05, time + dt + 0.01) }
              e.gain.setValueAtTime(0.7, time + 0.032); e.gain.exponentialRampToValueAtTime(0.0008, time + 0.22)
              play(n, 0.24); break
            }
            case 37: { // rim
              const o = osc('triangle', 1650); env(o.connect(filt('bandpass', 1650, 4)), 1.2, 0.03); play(o, 0.04)
              const n = nz(); env(n.connect(filt('highpass', 5000)), 0.3, 0.015); play(n, 0.02)
              break
            }
            case 42: case 44: case 46: { // hats (closed chokes open)
              const open = pitch === 46, d = open ? p.hatDecay * 7 : p.hatDecay
              if (!open && openHat) { openHat.gain.cancelScheduledValues(time); openHat.gain.setTargetAtTime(0, time, 0.005) }
              const n = nz()
              const e = env(n.connect(filt('highpass', p.hatTone)).connect(filt('peaking', p.hatTone * 1.2, 1)), open ? 0.45 : 0.5, d)
              if (open) openHat = e
              play(n, d); break
            }
            case 41: case 43: case 45: case 47: case 48: case 50: { // toms
              const f = p.tomTune * (pitch >= 48 ? 1.5 : pitch >= 45 ? 1 : 0.75)
              const o = osc('sine', f); o.frequency.exponentialRampToValueAtTime(f * 0.6, time + 0.35)
              env(o, 0.9, 0.4); play(o, 0.4); break
            }
            default: { // short pitched blip
              const o = osc('square', mtof(pitch)); env(o.connect(filt('lowpass', 3000)), 0.3, 0.12); play(o, 0.13)
            }
          }
        },
      }
    },
  }
}
