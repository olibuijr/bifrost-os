//! Musical generators: the Rust port of the core helpers of the mobile compose language (`server/compose.js`).
//! The mobile "compose" op ran model-written JavaScript; here the AI asks for native generators instead
//! (`generate` op in `ai.rs`): progressions from roman numerals, chord rolls, arpeggios, monophonic lines and
//! drum grooves. Tables and parsing rules follow compose.js.

use crate::model::{Extra, Note};

const NAMES: [(char, i32); 7] = [('C', 0), ('D', 2), ('E', 4), ('F', 5), ('G', 7), ('A', 9), ('B', 11)];

/// Scale intervals (compose.js SCALES).
pub fn scale(name: &str) -> &'static [i32] {
    match name {
        "major" => &[0, 2, 4, 5, 7, 9, 11],
        "dorian" => &[0, 2, 3, 5, 7, 9, 10],
        "phrygian" => &[0, 1, 3, 5, 7, 8, 10],
        "lydian" => &[0, 2, 4, 6, 7, 9, 11],
        "mixolydian" => &[0, 2, 4, 5, 7, 9, 10],
        "harm minor" => &[0, 2, 3, 5, 7, 8, 11],
        "mel minor" => &[0, 2, 3, 5, 7, 9, 11],
        "locrian" => &[0, 1, 3, 5, 6, 8, 10],
        "pent major" => &[0, 2, 4, 7, 9],
        "pent minor" => &[0, 3, 5, 7, 10],
        "blues" => &[0, 3, 5, 6, 7, 10],
        "chromatic" => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        _ => &[0, 2, 3, 5, 7, 8, 10],
    }
}

/// Chord qualities (compose.js QUAL).
pub fn quality(q: &str) -> &'static [i32] {
    match q {
        "" | "M" | "maj" => &[0, 4, 7],
        "m" | "min" => &[0, 3, 7],
        "dim" => &[0, 3, 6],
        "aug" => &[0, 4, 8],
        "sus2" => &[0, 2, 7],
        "sus4" => &[0, 5, 7],
        "5" => &[0, 7],
        "6" => &[0, 4, 7, 9],
        "m6" => &[0, 3, 7, 9],
        "7" => &[0, 4, 7, 10],
        "maj7" | "M7" => &[0, 4, 7, 11],
        "m7" => &[0, 3, 7, 10],
        "mM7" => &[0, 3, 7, 11],
        "dim7" => &[0, 3, 6, 9],
        "m7b5" => &[0, 3, 6, 10],
        "9" => &[0, 4, 7, 10, 14],
        "maj9" => &[0, 4, 7, 11, 14],
        "m9" => &[0, 3, 7, 10, 14],
        "add9" => &[0, 4, 7, 14],
        "madd9" => &[0, 3, 7, 14],
        "11" => &[0, 4, 7, 10, 14, 17],
        "m11" => &[0, 3, 7, 10, 14, 17],
        "13" => &[0, 4, 7, 10, 14, 21],
        "7b9" => &[0, 4, 7, 10, 13],
        "7#9" => &[0, 4, 7, 10, 15],
        "7sus4" => &[0, 5, 7, 10],
        "aug7" => &[0, 4, 8, 10],
        _ => &[0, 4, 7],
    }
}

/// Roman-numeral progressions (compose.js PROGS). Key-relative; plain numerals are diatonic to the song scale.
pub fn progression_preset(name: &str) -> Option<&'static str> {
    Some(match name {
        "andalusian" => "i VII VI V",
        "axis" | "pop" => "I V vi IV",
        "doowop" => "I vi IV V",
        "canon" => "I V vi iii IV I IV V",
        "lament" => "i v/3 iv/3 V",
        "trap_minor" | "epic" => "i VI III VII",
        "cinematic_mod" => "i bVI bIII bVII",
        "dub_minor" => "i i bVI bVII",
        "lofi_251" => "ii7 V7 Imaj7 vi7",
        "jazz_251" => "ii7 V7 Imaj7 Imaj7",
        "royal_road" => "IVmaj7 V7 iii7 vi7",
        "triphop" => "i7 bVImaj7 iv7 v7",
        "dark_hiphop" => "i bVI iv V",
        "dorian_vamp" => "i7 IV7",
        "phrygian" => "i bII i bII",
        "neo_soul" => "Imaj7 iii7 vi7 IVmaj7",
        "minor_blues" => "i7 i7 i7 i7 iv7 iv7 i7 i7 bVI7 V7 i7 V7",
        _ => return None,
    })
}

/// Parses a note name ("C#4", "Bb2", "E") or a number into a MIDI pitch (octave 4 when omitted).
pub fn midi(x: &str) -> Option<f64> {
    let s = x.trim();
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }
    let mut chars = s.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base = NAMES.iter().find(|(c, _)| *c == letter)?.1;
    let rest: String = chars.collect();
    let acc: String = rest.chars().take_while(|c| matches!(c, '#' | 'b' | '♯' | '♭')).collect();
    let mut p = base;
    for c in acc.chars() {
        p += if c == '#' || c == '♯' { 1 } else { -1 };
    }
    let oct_text = &rest[acc.len()..];
    let oct: i32 = if oct_text.is_empty() { 4 } else { oct_text.parse().ok()? };
    Some((p + oct * 12 + 12) as f64)
}

/// Pitches from a note list ("C3 G3 E4") or an array of names/numbers.
pub fn pitches(list: &str) -> Vec<f64> {
    list.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).filter_map(midi).collect()
}

/// Root pitch class, scale degree and accidental of a bare numeral ("V", "bVI", "ii7"). None if it is not a numeral.
fn numeral_root(sym: &str, key: i32, scale_name: &str) -> Option<(i32, usize, i32)> {
    let (acc, rest) = match sym.chars().next() {
        Some(c @ ('b' | '♭')) if sym.len() > c.len_utf8() => (-1, &sym[c.len_utf8()..]),
        Some(c @ ('#' | '♯')) => (1, &sym[c.len_utf8()..]),
        _ => (0, sym),
    };
    let numeral: String = rest.chars().take_while(|c| "iIvV".contains(*c)).collect();
    let degree = match numeral.to_ascii_lowercase().as_str() {
        "i" => 0,
        "ii" => 1,
        "iii" => 2,
        "iv" => 3,
        "v" => 4,
        "vi" => 5,
        "vii" => 6,
        _ => return None,
    };
    let pc = if acc != 0 {
        (key + scale("major")[degree] + acc + 24) % 12
    } else {
        (key + scale(scale_name).get(degree).copied().unwrap_or(0)) % 12
    };
    Some((pc, degree, acc))
}

/// Roman numeral -> chord pitches (octave `oct`). Supports accidentals (b, #), mods (° o ø +), extensions from
/// `quality`, slash bass ("iv/3" inversion as the third, "I/E" bass note) and secondary dominants ("V7/V", "ii7/V":
/// the numeral is built on the degree named after the slash, measured from that target's root).
pub fn roman(sym: &str, key: i32, scale_name: &str, oct: i32) -> Result<Vec<f64>, String> {
    let s = sym.trim();
    let (body, bass) = match s.split_once('/') {
        Some((b, bass)) => (b, Some(bass)),
        None => (s, None),
    };
    let (acc, rest) = match body.chars().next() {
        Some('b') | Some('♭') if body.len() > 1 => (-1, &body[body.chars().next().unwrap().len_utf8()..]),
        Some('#') | Some('♯') => (1, &body[body.chars().next().unwrap().len_utf8()..]),
        _ => (0, body),
    };
    let numeral: String = rest.chars().take_while(|c| "iIvV".contains(*c)).collect();
    if numeral.is_empty() {
        return Err(format!("bad roman numeral \"{sym}\""));
    }
    let ext_part = &rest[numeral.len()..];
    let (mod_char, ext) = match ext_part.chars().next() {
        Some(c @ ('°' | 'o' | 'ø' | '+')) => (Some(c), &ext_part[c.len_utf8()..]),
        _ => (None, ext_part),
    };
    let upper = numeral.chars().next().unwrap().is_uppercase();
    let degree = match numeral.to_ascii_lowercase().as_str() {
        "i" => 0,
        "ii" => 1,
        "iii" => 2,
        "iv" => 3,
        "v" => 4,
        "vi" => 5,
        "vii" => 6,
        _ => return Err(format!("bad roman numeral \"{sym}\"")),
    };
    let sc = scale(scale_name);
    let base_pc = if acc != 0 {
        (key + scale("major")[degree] + acc + 24) % 12
    } else {
        (key + sc.get(degree).copied().unwrap_or(0)) % 12
    };
    // secondary dominant: the root is the target degree's root plus this numeral's interval in the major scale
    let secondary = bass.and_then(|b| numeral_root(b, key, "major"));
    let base_pc = match secondary {
        Some((target_pc, _, _)) => (target_pc + scale("major")[degree] + acc + 24) % 12,
        None => base_pc,
    };
    let quality_name = match mod_char {
        Some('ø') => "m7b5".to_string(),
        Some('°') | Some('o') => if ext == "7" || ext == "dim7" { "dim7".into() } else { "dim".into() },
        Some('+') => if ext == "7" { "aug7".into() } else { "aug".into() },
        _ => {
            if upper || ext.starts_with('m') || ext.starts_with("sus") || ext.starts_with("dim") {
                ext.to_string()
            } else {
                match ext {
                    "" => "m".into(),
                    "7" => "m7".into(),
                    "9" => "m9".into(),
                    "6" => "m6".into(),
                    "11" | "13" => "m11".into(),
                    "add9" => "madd9".into(),
                    "maj7" | "maj9" => "mM7".into(),
                    other => format!("m{other}"),
                }
            }
        }
    };
    let root = ((base_pc % 12) + 12) % 12 + 12 * (oct + 1);
    let mut ps: Vec<f64> = quality(&quality_name).iter().map(|i| (root + i) as f64).collect();
    if let (Some(b), None) = (bass, secondary) {
        let inv_pc = match b.trim() {
            "3" => Some(ps.get(1).copied()),
            "5" => Some(ps.get(2).copied()),
            "7" => Some(ps.get(3).copied()),
            _ => None,
        };
        if let Some(Some(third)) = inv_pc {
            ps.retain(|p| *p != third);
            ps.insert(0, third - 12.0);
        } else if let Some(pitch) = midi(&format!("{b}{}", oct - 1)) {
            ps.retain(|p| *p != pitch);
            ps.insert(0, pitch);
        }
    }
    Ok(ps)
}

/// One chord per bar from a progression string ("i VI III VII" or a preset name).
pub fn progression(prog: &str, key: i32, scale_name: &str, bars: u32, oct: i32) -> Result<Vec<Vec<f64>>, String> {
    let text = progression_preset(prog).unwrap_or(prog);
    let symbols: Vec<&str> = text.split_whitespace().collect();
    if symbols.is_empty() {
        return Err("empty progression".into());
    }
    (0..bars as usize).map(|bar| roman(symbols[bar % symbols.len()], key, scale_name, oct)).collect()
}

/// Chord at `t`, lasting `d` steps, with an optional roll between notes (steps; negative rolls top-down).
pub fn chord(t: f64, pitches: &[f64], d: f64, v: f64, roll: f64) -> Vec<Note> {
    let order: Vec<f64> = if roll < 0.0 { pitches.iter().rev().copied().collect() } else { pitches.to_vec() };
    order
        .iter()
        .enumerate()
        .map(|(i, &p)| Note {
            t: t + roll.abs() * i as f64,
            d: (d - roll.abs() * i as f64).max(0.1),
            p,
            v: v.clamp(0.03, 1.0),
            prob: None,
            extra: Extra::new(),
        })
        .collect()
}

/// Arpeggio over chord tones: `rate` steps per note, `pattern` is "up", "down", "updown" or "downup".
pub fn arpeggio(t: f64, pitches: &[f64], count: usize, rate: f64, pattern: &str, v: f64) -> Vec<Note> {
    let list = pitches.to_vec();
    let order: Vec<f64> = match pattern {
        "down" => list.iter().rev().copied().collect(),
        "updown" => list.iter().copied().chain(list.iter().rev().skip(1).take(list.len().saturating_sub(2)).copied()).collect(),
        "downup" => list.iter().rev().copied().chain(list.iter().skip(1).take(list.len().saturating_sub(2)).copied()).collect(),
        _ => list.clone(),
    };
    if order.is_empty() {
        return vec![];
    }
    (0..count)
        .map(|i| Note { t: t + i as f64 * rate, d: rate, p: order[i % order.len()], v: v.clamp(0.03, 1.0), prob: None, extra: Extra::new() })
        .collect()
}

/// Monophonic line from "E5:2 D#5:1 r:1 E5:4" (pitch:duration in steps, r = rest). Returns the notes and the end step.
pub fn line(t: f64, text: &str, v: f64) -> (Vec<Note>, f64) {
    let mut at = t;
    let mut out = vec![];
    for tok in text.split_whitespace() {
        let (name, dur) = match tok.split_once(':') {
            Some((n, d)) => (n, parse_frac(d).unwrap_or(1.0)),
            None => (tok, 1.0),
        };
        if name != "r" && name != "-" {
            if let Some(p) = midi(name) {
                out.push(Note { t: at, d: dur * 0.95, p, v: v.clamp(0.03, 1.0), prob: None, extra: Extra::new() });
            }
        }
        at += dur;
    }
    (out, at)
}

fn parse_frac(s: &str) -> Option<f64> {
    match s.split_once('/') {
        Some((a, b)) => Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?),
        None => s.parse().ok(),
    }
    .filter(|v: &f64| *v > 0.0)
}

/// Drum pad notes (compose.js DRUM): kick 36, snare 38, clap 39, rim 37, closed hat 42, open hat 46, toms 45/50.
fn drum_pad(role: &str) -> Option<f64> {
    Some(match role {
        "kick" => 36.0,
        "snare" => 38.0,
        "clap" => 39.0,
        "rim" | "perc" => 37.0,
        "hat" => 42.0,
        "openhat" => 46.0,
        "tomLo" => 45.0,
        "tomHi" => 50.0,
        _ => return None,
    })
}

/// Drum groove (compose.js GROOVES): one string per role, one character per step: x hit, X accent, g ghost, . rest.
pub fn groove(name: &str, bars: u32) -> Option<Vec<Note>> {
    let roles: &[(&str, &str)] = match name {
        "boombap" => &[("kick", "X......x..X.....X......x..x..x.."), ("snare", "....X..g....X..."), ("hat", "x.x.x.x.x.x.x.xg"), ("openhat", "..............x.")],
        "trap" => &[("kick", "X.....x...x.....X.....x.x......x"), ("snare", "........X......."), ("clap", "........X......."), ("hat", "x.x.x.x.x.x.xxx.x.x.x.x.xxxxx.x.")],
        "triphop" => &[("kick", "X.......x.x.....X.........x....."), ("snare", "....X......g.X.g"), ("hat", "x.g.x.g.x.g.x.g."), ("perc", "......x.......x.")],
        "dubstep_halftime" => &[("kick", "X.........x.....X.....x........."), ("snare", "........X......."), ("hat", "x.x.x.x.x.x.x.x."), ("openhat", "......x.......x.")],
        "dnb" => &[("kick", "X.........X.....X.........x....."), ("snare", "....X.......X..g"), ("hat", "x.x.x.x.x.x.x.x.")],
        "house" => &[("kick", "X...X...X...X..."), ("clap", "....X.......X..."), ("hat", "g.g.g.g.g.g.g.g."), ("openhat", "..x...x...x...x.")],
        "lofi" => &[("kick", "X......x.x......X.........x....."), ("snare", "....X.......X..."), ("hat", "x.g.x.gxx.g.x.g.")],
        "reggaeton" => &[("kick", "X...X...X...X..."), ("snare", "...x..x....x..x."), ("hat", "x.x.x.x.x.x.x.x.")],
        "rock" => &[("kick", "X.....x.X.......X.....x.X.x....."), ("snare", "....X.......X..."), ("hat", "X.x.X.x.X.x.X.x.")],
        _ => return None,
    };
    let mut out = vec![];
    for (role, pattern) in roles {
        let pitch = drum_pad(role)?;
        let chars: Vec<char> = pattern.chars().collect();
        for bar in 0..bars as usize {
            for (i, c) in chars.iter().enumerate() {
                let (vel, hit) = match c {
                    'x' => (0.8, true),
                    'X' => (1.0, true),
                    'g' => (0.35, true),
                    _ => (0.0, false),
                };
                if hit {
                    let t = (bar * 16) as f64 + i as f64 * (16.0 / chars.len() as f64);
                    out.push(Note { t, d: 1.0, p: pitch, v: vel, prob: None, extra: Extra::new() });
                }
            }
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_map_to_midi() {
        assert_eq!(midi("C4"), Some(60.0));
        assert_eq!(midi("A4"), Some(69.0));
        assert_eq!(midi("Bb3"), Some(58.0));
    }

    #[test]
    fn roman_numerals_build_triads() {
        let i = roman("i", 0, "minor", 4).unwrap();
        assert_eq!(i, vec![60.0, 63.0, 67.0]);
        let v7 = roman("V7", 0, "minor", 4).unwrap();
        assert!(v7.len() >= 3);
    }

    #[test]
    fn progression_cycles_over_bars() {
        let chords = progression("i VI III VII", 0, "minor", 6, 3).unwrap();
        assert_eq!(chords.len(), 6);
        assert_eq!(chords[0], chords[4]);
    }
}

/// Post-processing helpers (compose.js humanize, dynamics, pedal, pedalEvery, near, pcNear).
/// Deterministic jitter: the same seed gives the same result (the mobile composer seeded its PRNG too).
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed.max(1))
    }
    /// Uniform in 0..1 (xorshift32).
    pub fn unit(&mut self) -> f64 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        s as f64 / 4_294_967_296.0
    }
    /// Approximately normal, mean 0, spread about +-1 (sum of three uniforms, as in compose.js gauss).
    pub fn gauss(&mut self) -> f64 {
        (self.unit() + self.unit() + self.unit() - 1.5) / 1.5
    }
}

/// Timing and velocity jitter; downbeats are kept tighter.
pub fn humanize(notes: &mut [Note], length: f64, time: f64, vel: f64, seed: u32) {
    let mut rng = Rng::new(seed);
    for n in notes.iter_mut() {
        let on_beat = (n.t % 4.0).abs() < 1e-6;
        n.t = (n.t + rng.gauss() * time * if on_beat { 0.5 } else { 1.0 }).clamp(0.0, length - 0.01);
        n.v = (n.v * (1.0 + rng.gauss() * vel)).clamp(0.03, 1.0);
    }
}

/// Linear velocity envelope over steps: points are (step, factor).
pub fn dynamics(notes: &mut [Note], points: &[(f64, f64)]) {
    if points.is_empty() {
        return;
    }
    let factor = |t: f64| -> f64 {
        if t <= points[0].0 {
            return points[0].1;
        }
        for w in points.windows(2) {
            let ((a, va), (b, vb)) = (w[0], w[1]);
            if t <= b {
                return va + (vb - va) * (t - a) / (b - a).max(1e-9);
            }
        }
        points[points.len() - 1].1
    };
    for n in notes.iter_mut() {
        n.v = (n.v * factor(n.t)).clamp(0.03, 1.0);
    }
}

/// Sustain pedal: notes starting in [from, to) ring until `to`.
pub fn pedal(notes: &mut [Note], from: f64, to: f64, max_pitch: f64, gap: f64) {
    for n in notes.iter_mut() {
        if n.t >= from && n.t < to && n.p <= max_pitch {
            n.d = n.d.max(to - n.t - gap);
        }
    }
}

/// Pedal changes every `every` steps across the clip.
pub fn pedal_every(notes: &mut [Note], length: f64, every: f64) {
    let mut a = 0.0;
    while a < length {
        pedal(notes, a, (a + every).min(length), 127.0, 0.1);
        a += every;
    }
}

/// Moves each pitch to the octave nearest the reference chord's average (voice leading), then sorts.
pub fn near(target: &[f64], reference: &[f64]) -> Vec<f64> {
    let centre = reference.iter().sum::<f64>() / reference.len().max(1) as f64;
    let mut out: Vec<f64> = target
        .iter()
        .map(|&p| {
            let mut q = p;
            while q - centre > 6.0 {
                q -= 12.0;
            }
            while centre - q > 6.0 {
                q += 12.0;
            }
            q
        })
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out
}

const PCN: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
/// Chord qualities tried by `harmony` (compose.js CHORD_Q).
const CHORD_Q: &[(&str, &[i32])] = &[
    ("", &[0, 4, 7]),
    ("m", &[0, 3, 7]),
    ("dim", &[0, 3, 6]),
    ("aug", &[0, 4, 8]),
    ("7", &[0, 4, 7, 10]),
    ("maj7", &[0, 4, 7, 11]),
    ("m7", &[0, 3, 7, 10]),
    ("m7b5", &[0, 3, 6, 10]),
    ("dim7", &[0, 3, 6, 9]),
    ("sus4", &[0, 5, 7]),
    ("5", &[0, 7]),
];

/// Harmony of the notes sounding in a window (compose.js harmonyOf).
#[derive(Debug, Clone, PartialEq)]
pub struct Harmony {
    pub empty: bool,
    pub root: Option<i32>,
    pub quality: &'static str,
    pub name: String,
    pub chord_tones: Vec<i32>,
    /// Pitch classes with real weight, strongest first.
    pub tones: Vec<i32>,
    /// The same pitch classes, ascending.
    pub pcs: Vec<i32>,
    /// Lowest sounding pitch.
    pub bass: Option<f64>,
    pub bass_pc: Option<i32>,
}

fn no_chord() -> Harmony {
    Harmony { empty: true, root: None, quality: "", name: "N.C.".into(), chord_tones: vec![], tones: vec![], pcs: vec![], bass: None, bass_pc: None }
}

fn pc_of(p: f64) -> usize {
    (p.round() as i64).rem_euclid(12) as usize
}

/// Duration x velocity weighted pitch classes of the notes sounding in [from, to), the bass note and the best-fit chord.
pub fn harmony(notes: &[Note], from: f64, to: f64) -> Harmony {
    let mut w = [0.0f64; 12];
    let mut bass: Option<f64> = None;
    for n in notes {
        let a = from.max(n.t);
        let b = to.min(n.t + n.d);
        if b <= a {
            continue;
        }
        w[pc_of(n.p)] += (b - a) * (0.4 + n.v);
        if bass.map_or(true, |p| n.p < p) {
            bass = Some(n.p);
        }
    }
    let total: f64 = w.iter().sum();
    if !(total > 0.0) {
        return no_chord();
    }
    let bass_pc = bass.map(|p| pc_of(p) as i32);
    let mut best: Option<(f64, i32, &'static str, &'static [i32])> = None;
    for r in 0..12i32 {
        for &(q, iv) in CHORD_Q {
            let tones: Vec<i32> = iv.iter().map(|i| (r + i) % 12).collect();
            let inside: f64 = tones.iter().map(|&t| w[t as usize]).sum();
            let outside: f64 = (0..12).filter(|i| !tones.contains(i)).map(|i| w[i as usize] * 0.7).sum();
            let bonus = if bass_pc == Some(r) { total * 0.08 } else { 0.0 };
            let sc = inside - outside + bonus - iv.len() as f64 * total * 0.01;
            if best.map_or(true, |b| sc > b.0) {
                best = Some((sc, r, q, iv));
            }
        }
    }
    let Some((_, root, quality, iv)) = best else { return no_chord() };
    let chord_tones: Vec<i32> = iv.iter().map(|i| (root + i) % 12).collect();
    let mut order: Vec<usize> = (0..12).filter(|&i| w[i] > total * 0.04).collect();
    order.sort_by(|&a, &b| w[b].partial_cmp(&w[a]).unwrap_or(std::cmp::Ordering::Equal));
    let tones: Vec<i32> = order.iter().map(|&i| i as i32).collect();
    let mut pcs = tones.clone();
    pcs.sort();
    let slash = match bass_pc {
        Some(b) if b != root => format!("/{}", PCN[b as usize]),
        _ => String::new(),
    };
    Harmony {
        empty: false,
        root: Some(root),
        quality,
        name: format!("{}{}{}", PCN[root as usize], quality, slash),
        chord_tones,
        tones,
        pcs,
        bass,
        bass_pc,
    }
}

/// One harmony per window of `every` steps over [0, length) (compose.js chordMap).
pub fn chord_map(notes: &[Note], length: f64, every: f64) -> Vec<(f64, Harmony)> {
    if !(every > 0.0) {
        return vec![];
    }
    let mut out = vec![];
    let mut a = 0.0;
    while a < length {
        out.push((a, harmony(notes, a, a + every)));
        a += every;
    }
    out
}

/// Pitch with pitch class `pc` at or below `reference`; dir 0 moves it up an octave when that is nearer,
/// dir > 0 forces it at or above the reference (compose.js pcNear).
pub fn pc_near(pc: f64, reference: f64, dir: f64) -> f64 {
    let mut p = reference - (((reference - pc) % 12.0) + 12.0) % 12.0;
    if dir == 0.0 && reference - p > 6.0 {
        p += 12.0;
    }
    if dir > 0.0 && p < reference {
        p += 12.0;
    }
    p
}

#[cfg(test)]
mod harmony_tests {
    use super::*;

    fn note(t: f64, d: f64, p: f64) -> Note {
        Note { t, d, p, v: 0.8, prob: None, extra: Extra::new() }
    }

    #[test]
    fn secondary_dominants_build_on_the_target_degree() {
        assert_eq!(roman("V/V", 0, "major", 4).unwrap(), vec![62.0, 66.0, 69.0]);
        assert_eq!(roman("V7/V", 0, "major", 4).unwrap(), vec![62.0, 66.0, 69.0, 72.0]);
        assert_eq!(roman("ii7/V", 0, "major", 4).unwrap(), vec![69.0, 72.0, 76.0, 79.0]);
    }

    #[test]
    fn pc_near_picks_the_closest_octave() {
        assert_eq!(pc_near(4.0, 60.0, 0.0), 64.0);
    }

    #[test]
    fn harmony_names_a_major_triad() {
        let ns = [note(0.0, 16.0, 60.0), note(0.0, 16.0, 64.0), note(0.0, 16.0, 67.0)];
        let h = harmony(&ns, 0.0, 16.0);
        assert_eq!(h.name, "C");
        assert_eq!(chord_map(&ns, 32.0, 16.0).len(), 2);
    }
}

#[cfg(test)]
mod post_tests {
    use super::*;

    #[test]
    fn near_moves_pitches_into_the_reference_register() {
        let out = near(&[72.0, 40.0], &[60.0, 64.0, 67.0]);
        assert!(out.iter().all(|p| (p - 63.0).abs() <= 12.0));
    }

    #[test]
    fn pedal_extends_notes_to_the_release_point() {
        let mut ns = vec![Note { t: 0.0, d: 1.0, p: 60.0, v: 0.8, prob: None, extra: Extra::new() }];
        pedal(&mut ns, 0.0, 16.0, 127.0, 0.1);
        assert!(ns[0].d > 15.0);
    }
}
