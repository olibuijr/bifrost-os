//! `bifrost-unit`: inspects and renders audio units from a shell, so a plug-in library can be checked without an app.
//! `scan` loads the plug-in libraries of the default directories (or the ones named) and prints what each says about
//! itself and its parameters. `render` runs one unit (a library path, or a built-in: `gain`, `biquad`, `sine`, `gm`)
//! through the audio engine offline and writes a 16-bit WAV file: an instrument plays one note through a sequencer, an
//! effect processes a 440 Hz tone. The logic is here so it can be unit tested; the binary parses, loads and writes.

use bifrost_app::audio_engine::units::plugin::{default_directories, scan_directory, PluginInfo, PluginLibrary};
use bifrost_app::audio_engine::units::{
    AudioUnit, ComponentDescription, ComponentRegistry, ParamSpec, SeqEvent, Sequence, SequencerUnit, UnitKind,
};
use bifrost_app::audio_engine::{Engine, EngineConfig, OUTPUT};
use bifrost_app::midi_player::{wav_bytes_from_planar, MidiPlayer};
use std::path::PathBuf;

pub const USAGE: &str = "usage: bifrost-unit scan [DIRECTORY...]\n       bifrost-unit render UNIT [--seconds N] [--rate HZ] [--note KEY] [--set ID=VALUE]... OUTPUT.wav\n       bifrost-unit midi FILE.mid OUTPUT.wav [--rate HZ]\n\nmidi plays a Standard MIDI File through the built-in General MIDI synth into a WAV file.\nUNIT is a plug-in library (a path ending in .so) or a built-in: gain, biquad, sine, gm.\nscan prints each plug-in found in the directories (default: /usr/lib/bifrost/units and ~/.local/share/bifrost/units).\nrender plays one note of an instrument (default 60) or processes a 440 Hz tone for an effect; 2 seconds at 48000 Hz unless told.";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Help,
    Scan { dirs: Vec<PathBuf> },
    Render { unit: String, seconds: f64, rate: f64, note: u8, sets: Vec<(u32, f32)>, out: PathBuf },
    Midi { file: PathBuf, out: PathBuf, rate: f64 },
}

/// The value after an option: moves `i` onto it, or says the option needs one.
fn next_arg<'a>(rest: &'a [String], i: &mut usize, name: &str) -> Result<&'a String, String> {
    *i += 1;
    rest.get(*i).ok_or_else(|| format!("{name} needs a value"))
}

pub fn parse(args: &[String]) -> Result<Plan, String> {
    let Some(cmd) = args.first() else { return Err(USAGE.to_string()) };
    let rest = &args[1..];
    match cmd.as_str() {
        "-h" | "--help" | "help" => Ok(Plan::Help),
        "scan" => Ok(Plan::Scan { dirs: rest.iter().map(PathBuf::from).collect() }),
        "midi" => {
            let mut files = Vec::new();
            let mut rate = 48_000.0f64;
            let mut i = 0;
            while i < rest.len() {
                if rest[i] == "--rate" {
                    let v = rest.get(i + 1).ok_or_else(|| "--rate needs a value".to_string())?;
                    rate = v.parse::<f64>().ok().filter(|r| (8_000.0..=192_000.0).contains(r)).ok_or_else(|| format!("`{v}` is not a sample rate (8000 to 192000)"))?;
                    i += 2;
                } else if rest[i].starts_with("--") {
                    return Err(format!("unknown option {}", rest[i]));
                } else {
                    files.push(PathBuf::from(&rest[i]));
                    i += 1;
                }
            }
            match files.as_slice() {
                [file, out] => Ok(Plan::Midi { file: file.clone(), out: out.clone(), rate }),
                _ => Err(USAGE.to_string()),
            }
        }
        "render" => {
            let mut unit = None;
            let mut out = None;
            let (mut seconds, mut rate, mut note, mut sets) = (2.0f64, 48_000.0f64, 60u8, Vec::new());
            let mut i = 0;
            while i < rest.len() {
                let a = rest[i].as_str();
                match a {
                    "--seconds" => {
                        let v = next_arg(rest, &mut i, "--seconds")?;
                        seconds = v.parse::<f64>().ok().filter(|s| s.is_finite() && *s > 0.0 && *s <= 600.0).ok_or_else(|| format!("`{v}` is not a length in seconds (0 to 600)"))?;
                    }
                    "--rate" => {
                        let v = next_arg(rest, &mut i, "--rate")?;
                        rate = v.parse::<f64>().ok().filter(|r| (8_000.0..=192_000.0).contains(r)).ok_or_else(|| format!("`{v}` is not a sample rate (8000 to 192000)"))?;
                    }
                    "--note" => {
                        let v = next_arg(rest, &mut i, "--note")?;
                        note = v.parse::<u8>().ok().filter(|k| *k < 128).ok_or_else(|| format!("`{v}` is not a key number 0 to 127"))?;
                    }
                    "--set" => {
                        let v = next_arg(rest, &mut i, "--set")?;
                        let (id, val) = v.split_once('=').ok_or_else(|| format!("`{v}` is not ID=VALUE"))?;
                        let id = id.parse::<u32>().map_err(|_| format!("`{id}` is not a parameter id"))?;
                        let val = val.parse::<f32>().ok().filter(|x| x.is_finite()).ok_or_else(|| format!("`{val}` is not a number"))?;
                        sets.push((id, val));
                    }
                    flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
                    positional => {
                        if unit.is_none() {
                            unit = Some(positional.to_string());
                        } else if out.is_none() {
                            out = Some(PathBuf::from(positional));
                        } else {
                            return Err(USAGE.to_string());
                        }
                    }
                }
                i += 1;
            }
            match (unit, out) {
                (Some(unit), Some(out)) => Ok(Plan::Render { unit, seconds, rate, note, sets, out }),
                _ => Err(USAGE.to_string()),
            }
        }
        _ => Err(USAGE.to_string()),
    }
}

/// The four-character code as text (a code that is not printable shows as `????`).
pub fn code_text(code: u32) -> String {
    let b = code.to_be_bytes();
    if b.iter().all(|c| c.is_ascii_graphic()) {
        String::from_utf8_lossy(&b).into_owned()
    } else {
        "????".to_string()
    }
}

fn kind_text(k: UnitKind) -> &'static str {
    match k {
        UnitKind::Effect => "effect",
        UnitKind::Instrument => "instrument",
        UnitKind::Generator => "generator",
    }
}

/// What `scan` prints for a loaded plug-in.
pub fn describe(info: &PluginInfo, specs: &[ParamSpec]) -> String {
    let d: ComponentDescription = info.description;
    let mut out = format!(
        "{}  {}  {}/{}/{}  up to {} channels{}  {} parameter{}\n",
        info.name,
        kind_text(info.kind),
        code_text(d.kind),
        code_text(d.sub_kind),
        code_text(d.manufacturer),
        info.max_channels,
        if info.accepts_midi { ", MIDI" } else { "" },
        specs.len(),
        if specs.len() == 1 { "" } else { "s" }
    );
    for s in specs {
        let group = if s.group.is_empty() { String::new() } else { format!("{}/", s.group) };
        out.push_str(&format!("  {:>3}  {}{}  {:?}  {} to {}  default {}{}\n", s.id, group, s.name, s.unit, s.min, s.max, s.default, if s.read_only { "  (read-only)" } else { "" }));
    }
    out
}

/// Loads `spec`: a library path (ending in `.so`), or a built-in by short name.
pub fn load_unit(spec: &str) -> Result<Box<dyn AudioUnit>, String> {
    if spec.ends_with(".so") {
        let lib = PluginLibrary::open(std::path::Path::new(spec)).map_err(|e| e.to_string())?;
        return lib.instantiate().map(|u| Box::new(u) as Box<dyn AudioUnit>).map_err(|e| e.to_string());
    }
    let (name, sub) = match spec {
        "gain" => ("Gain", "gain"),
        "biquad" => ("Biquad Filter", "bqad"),
        "sine" => ("Sine Synth", "sine"),
        "gm" => ("General MIDI Synth", "gmsy"),
        other => return Err(format!("`{other}` is not a plug-in library (.so) or a built-in unit (gain, biquad, sine, gm)")),
    };
    let reg = ComponentRegistry::with_builtins();
    let at = (0..reg.len()).find(|i| reg.name(*i) == Some(name)).ok_or_else(|| format!("built-in {sub} is missing"))?;
    reg.instantiate(at).map_err(|e| e.to_string())
}

/// Renders one unit through the engine, offline: an instrument plays `note` for three quarters of `seconds` through a
/// sequencer (and rings out for the rest), an effect processes a 440 Hz sine of `seconds`. The parameters in `sets`
/// are applied first. Returns the planar channels.
pub fn render(unit: Box<dyn AudioUnit>, seconds: f64, rate: f64, note: u8, sets: &[(u32, f32)]) -> Result<Vec<Vec<f32>>, String> {
    let channels = unit.info().max_channels.min(2).max(1);
    let kind = unit.info().kind;
    let mut engine = Engine::new(EngineConfig { sample_rate: rate, channels, max_frames: 512 }).map_err(|e| e.to_string())?;
    let frames = (seconds * rate).ceil() as usize;
    match kind {
        UnitKind::Effect => {
            let tone: Vec<f32> = (0..frames)
                .flat_map(|i| {
                    let s = 0.5 * (std::f64::consts::TAU * 440.0 * i as f64 / rate).sin() as f32;
                    std::iter::repeat(s).take(channels as usize)
                })
                .collect();
            let (player, handle) = engine.attach_player(rate, channels, &tone).map_err(|e| e.to_string())?;
            let (node, params, _) = engine.attach_unit(unit).map_err(|e| e.to_string())?;
            for (id, v) in sets {
                params.set(*id, *v).map_err(|e| format!("parameter {id}: {e}"))?;
            }
            engine.connect(player, node).map_err(|e| e.to_string())?;
            engine.connect(node, OUTPUT).map_err(|e| e.to_string())?;
            handle.play();
        }
        UnitKind::Instrument | UnitKind::Generator => {
            let sequence = Sequence::new(vec![SeqEvent::note_on(0.0, 0, note, 100), SeqEvent::note_off(seconds * 0.75, 0, note)]);
            let (sequenced, control) = SequencerUnit::new(unit, sequence);
            let (node, params, _) = engine.attach_unit(Box::new(sequenced)).map_err(|e| e.to_string())?;
            for (id, v) in sets {
                params.set(*id, *v).map_err(|e| format!("parameter {id}: {e}"))?;
            }
            engine.connect(node, OUTPUT).map_err(|e| e.to_string())?;
            control.play();
        }
    }
    engine.render_offline(frames).map_err(|e| e.to_string())
}

/// A Standard MIDI File played through the built-in synth, as WAV bytes (the last notes ring out for half a second).
pub fn render_midi(bytes: &[u8], rate: f64) -> Result<Vec<u8>, String> {
    let mut player = MidiPlayer::from_bytes(bytes, rate).map_err(|e| e.to_string())?;
    player.render_to_wav(0.5).map_err(|e| e.to_string())
}

/// The WAV bytes for a render.
pub fn wav(planes: &[Vec<f32>], rate: f64) -> Vec<u8> {
    wav_bytes_from_planar(rate as u32, planes)
}

/// Every plug-in in `dirs` (the default directories when empty): the text `scan` prints, and how many loaded.
pub fn scan(dirs: &[PathBuf]) -> (String, usize) {
    let dirs: Vec<PathBuf> = if dirs.is_empty() { default_directories() } else { dirs.to_vec() };
    let mut out = String::new();
    let mut loaded = 0;
    for dir in dirs {
        let mut registry = ComponentRegistry::new();
        for report in scan_directory(&dir, &mut registry) {
            match report.result {
                Ok(_) => {
                    loaded += 1;
                    // the library is loaded again to read its description (the registry keeps only the factory)
                    match PluginLibrary::open(&report.path) {
                        Ok(lib) => {
                            out.push_str(&format!("{}\n", report.path.display()));
                            out.push_str(&describe(lib.info(), lib.param_specs()));
                        }
                        Err(e) => out.push_str(&format!("{}: {e}\n", report.path.display())),
                    }
                }
                Err(e) => out.push_str(&format!("{}: {e}\n", report.path.display())),
            }
        }
    }
    if out.is_empty() {
        out.push_str("no plug-in libraries found\n");
    }
    (out, loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn the_command_line_is_parsed() {
        assert_eq!(parse(&args(&["scan"])), Ok(Plan::Scan { dirs: vec![] }));
        assert_eq!(parse(&args(&["scan", "/a", "/b"])), Ok(Plan::Scan { dirs: vec![PathBuf::from("/a"), PathBuf::from("/b")] }));
        assert_eq!(
            parse(&args(&["render", "gm", "out.wav"])),
            Ok(Plan::Render { unit: "gm".into(), seconds: 2.0, rate: 48_000.0, note: 60, sets: vec![], out: PathBuf::from("out.wav") })
        );
        assert_eq!(
            parse(&args(&["render", "--note", "72", "x.so", "--seconds", "1.5", "--rate", "44100", "--set", "0=0.5", "--set", "3=-2", "o.wav"])),
            Ok(Plan::Render { unit: "x.so".into(), seconds: 1.5, rate: 44_100.0, note: 72, sets: vec![(0, 0.5), (3, -2.0)], out: PathBuf::from("o.wav") })
        );
        assert_eq!(parse(&args(&["--help"])), Ok(Plan::Help));
    }

    #[test]
    fn bad_command_lines_say_what_is_wrong() {
        assert!(parse(&[]).is_err());
        assert!(parse(&args(&["render", "gm"])).is_err());
        assert!(parse(&args(&["render", "gm", "a.wav", "b.wav"])).is_err());
        assert!(parse(&args(&["render", "gm", "a.wav", "--seconds"])).unwrap_err().contains("needs a value"));
        assert!(parse(&args(&["render", "gm", "a.wav", "--seconds", "0"])).unwrap_err().contains("seconds"));
        assert!(parse(&args(&["render", "gm", "a.wav", "--rate", "5"])).unwrap_err().contains("sample rate"));
        assert!(parse(&args(&["render", "gm", "a.wav", "--note", "200"])).unwrap_err().contains("key"));
        assert!(parse(&args(&["render", "gm", "a.wav", "--set", "x"])).unwrap_err().contains("ID=VALUE"));
        assert!(parse(&args(&["render", "gm", "a.wav", "--set", "a=1"])).unwrap_err().contains("parameter id"));
        assert!(parse(&args(&["render", "gm", "a.wav", "--frobnicate"])).unwrap_err().contains("unknown option"));
        assert!(parse(&args(&["fly"])).is_err());
    }

    #[test]
    fn component_codes_print_as_text() {
        assert_eq!(code_text(u32::from_be_bytes(*b"aufx")), "aufx");
        assert_eq!(code_text(0), "????");
    }

    #[test]
    fn a_plugin_description_lists_its_parameters() {
        let info = PluginInfo {
            name: "Test Gain".into(),
            kind: UnitKind::Effect,
            accepts_midi: false,
            max_channels: 2,
            description: ComponentDescription::new("aufx", "tgan", "test").unwrap(),
            param_count: 2,
        };
        let specs = vec![
            ParamSpec::new(0, "Gain", bifrost_app::audio_engine::units::ParamUnit::Ratio, 0.0, 4.0, 1.0).unwrap().in_group("Main"),
            ParamSpec::new(1, "Meter", bifrost_app::audio_engine::units::ParamUnit::Generic, 0.0, 1.0, 0.0).unwrap().read_only(),
        ];
        let text = describe(&info, &specs);
        assert!(text.starts_with("Test Gain  effect  aufx/tgan/test  up to 2 channels  2 parameters\n"), "{text}");
        assert!(text.contains("Main/Gain"));
        assert!(text.contains("(read-only)"));
    }

    #[test]
    fn built_in_units_load_by_name_and_bad_names_are_refused() {
        for (name, unit) in [("gain", "Gain"), ("biquad", "Biquad Filter"), ("sine", "Sine Synth"), ("gm", "General MIDI Synth")] {
            assert_eq!(load_unit(name).unwrap().info().name, unit);
        }
        assert!(load_unit("reverb").err().unwrap().contains("built-in"));
        assert!(load_unit("/nonexistent/x.so").err().unwrap().contains("cannot load"));
    }

    #[test]
    fn an_instrument_renders_a_note_that_ends_and_rings_out() {
        let planes = render(load_unit("gm").unwrap(), 1.0, 48_000.0, 69, &[]).unwrap();
        assert_eq!(planes.len(), 2);
        assert_eq!(planes[0].len(), 48_000);
        assert!(rms(&planes[0][2_400..30_000]) > 0.005);
        // the note-off is at 0.75 s and the release is 150 ms: silent for the last tenth of a second
        assert!(planes[0][43_200..].iter().all(|s| s.abs() < 1e-5));
    }

    #[test]
    fn an_effect_processes_the_tone_and_its_parameters_apply() {
        let normal = render(load_unit("gain").unwrap(), 0.5, 48_000.0, 60, &[]).unwrap();
        let silent = render(load_unit("gain").unwrap(), 0.5, 48_000.0, 60, &[(0, 0.0)]).unwrap();
        let double = render(load_unit("gain").unwrap(), 0.5, 48_000.0, 60, &[(0, 2.0)]).unwrap();
        assert!((rms(&normal[0][2_000..]) - 0.5 / 2f32.sqrt()).abs() < 0.01, "{}", rms(&normal[0][2_000..]));
        assert!(rms(&silent[0][2_000..]) < 1e-6);
        assert!((rms(&double[0][2_000..]) / rms(&normal[0][2_000..]) - 2.0).abs() < 0.02);
        assert!(render(load_unit("gain").unwrap(), 0.5, 48_000.0, 60, &[(9, 1.0)]).unwrap_err().contains("parameter 9"));
    }

    #[test]
    fn the_wav_has_the_rate_and_channels_of_the_render() {
        let planes = render(load_unit("sine").unwrap(), 0.25, 44_100.0, 60, &[]).unwrap();
        let w = wav(&planes, 44_100.0);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([w[24], w[25], w[26], w[27]]), 44_100);
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), planes.len() as u16);
    }

    /// A format 0 file, 480 ticks per quarter, 120 bpm: middle C for one beat.
    fn smf() -> Vec<u8> {
        let mut track = vec![0x00, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 0, 0x00, 0xFF, 0x2F, 0x00];
        let mut file = b"MThd".to_vec();
        file.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0x01, 0xE0]);
        file.extend_from_slice(b"MTrk");
        file.extend_from_slice(&(track.len() as u32).to_be_bytes());
        file.append(&mut track);
        file
    }

    #[test]
    fn midi_files_render_to_a_wav_of_the_right_length() {
        let wav = render_midi(&smf(), 24_000.0).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        // 0.5 s of music, 0.5 s of tail, and one frame, stereo 16-bit
        let data = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(data, (24_000 + 1) * 4);
        assert!(wav[44..].iter().any(|b| *b != 0));
        assert!(render_midi(b"nope", 24_000.0).is_err());
    }

    #[test]
    fn the_midi_command_line_is_parsed() {
        assert_eq!(parse(&args(&["midi", "a.mid", "o.wav"])), Ok(Plan::Midi { file: PathBuf::from("a.mid"), out: PathBuf::from("o.wav"), rate: 48_000.0 }));
        assert_eq!(
            parse(&args(&["midi", "--rate", "22050", "a.mid", "o.wav"])),
            Ok(Plan::Midi { file: PathBuf::from("a.mid"), out: PathBuf::from("o.wav"), rate: 22_050.0 })
        );
        assert!(parse(&args(&["midi", "a.mid"])).is_err());
        assert!(parse(&args(&["midi", "a.mid", "o.wav", "--rate"])).unwrap_err().contains("needs a value"));
        assert!(parse(&args(&["midi", "a.mid", "o.wav", "--rate", "9"])).unwrap_err().contains("sample rate"));
        assert!(parse(&args(&["midi", "a.mid", "o.wav", "--loud"])).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn scanning_a_missing_directory_finds_nothing() {
        let (text, n) = scan(&[PathBuf::from("/nonexistent/bifrost-units")]);
        assert_eq!(n, 0);
        assert_eq!(text, "no plug-in libraries found\n");
    }
}
