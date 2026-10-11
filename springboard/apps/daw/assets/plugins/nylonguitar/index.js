// Generic sampled instrument: everything (name, range, zones, envelope) comes from plugin.json + zones.json
// in this folder; playback is the shared host.multisample engine. Reusable as-is for any sample set.
export default function plugin(host) {
  const meta = host.plugin
  const ms = host.multisample("zones.json", { defaults: meta.defaults || {} })
  return {
    type: meta.type, name: meta.name, desc: meta.desc, family: meta.family, range: meta.range, sampled: "multi",
    params: ms.params, create: ms.create, preload: ms.preload,
  }
}
