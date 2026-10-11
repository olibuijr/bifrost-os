// Strings — multisampled instrument: zones.json + audio files in this folder, played by host.multisample.
export default function plugin(host) {
  const ms = host.multisample('zones.json', { defaults: { vibrato: 8, detune: 5, stereo: 0.35 } })
  return {
    type: 'strings', name: 'Strings', desc: host.plugin.desc, family: 'strings', range: host.plugin.range, sampled: 'multi',
    params: ms.params, create: ms.create, preload: ms.preload,
  }
}
