//! App-facing font objects: a `FontSpec` describes what an app wants (names, size, traits, weight, width, slant,
//! matrix, variations), a `FontManager` holds the registered and installed faces and matches specs against them,
//! a `FontSet` is a named or matched group of faces, and a `FontObject` is one face at one size with metrics, glyph
//! lookup, tables and a fallback cascade. The manager never draws; the renderer in `font.rs` does that for the
//! bytes a `FontObject` hands it.

use crate::font::{Font, Style, Var};
use crate::fontinfo::{FaceInfo, GlyphTables, NameId, Tables, Traits};
use crate::gfx::Image;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A value in the attribute listing of a spec.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    Text(String),
    Number(f32),
    Traits(Traits),
    Matrix([f32; 6]),
    Variations(Vec<([u8; 4], f32)>),
}

pub const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// What an app asks for. Every field is optional; a set name is a hard filter, traits, weight, width and slant only
/// rank the candidates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontSpec {
    pub family: Option<String>,
    pub style: Option<String>,
    pub full_name: Option<String>,
    pub postscript_name: Option<String>,
    pub size: Option<f32>,
    /// wanted trait bits, only the bits in `trait_mask` count
    pub traits: Traits,
    pub trait_mask: u32,
    /// -1.0 (thin) to 1.0 (black), 0.0 Regular
    pub weight: Option<f32>,
    /// -1.0 (condensed) to 1.0 (expanded)
    pub width: Option<f32>,
    /// slant in degrees clockwise from upright
    pub slant: Option<f32>,
    pub matrix: Option<[f32; 6]>,
    pub variations: Vec<([u8; 4], f32)>,
    /// family names tried, in order, for characters the face lacks
    pub cascade: Vec<String>,
}

impl FontSpec {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn family(mut self, n: &str) -> Self {
        self.family = Some(n.to_string());
        self
    }
    pub fn style(mut self, n: &str) -> Self {
        self.style = Some(n.to_string());
        self
    }
    pub fn full_name(mut self, n: &str) -> Self {
        self.full_name = Some(n.to_string());
        self
    }
    pub fn postscript_name(mut self, n: &str) -> Self {
        self.postscript_name = Some(n.to_string());
        self
    }
    /// Sets the size; a value that is not finite and above zero clears it.
    pub fn size(mut self, pt: f32) -> Self {
        self.size = (pt.is_finite() && pt > 0.0).then_some(pt);
        self
    }
    pub fn weight(mut self, w: f32) -> Self {
        self.weight = Some(w.clamp(-1.0, 1.0));
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w.clamp(-1.0, 1.0));
        self
    }
    pub fn slant(mut self, deg: f32) -> Self {
        self.slant = Some(deg);
        self
    }
    pub fn matrix(mut self, m: [f32; 6]) -> Self {
        self.matrix = Some(m);
        self
    }
    pub fn variation(mut self, tag: &[u8; 4], v: f32) -> Self {
        self.variations.retain(|x| &x.0 != tag);
        self.variations.push((*tag, v));
        self
    }
    /// Asks for the trait bits in `bits` to be on (or off) and no others to matter.
    pub fn traits(mut self, bits: u32, on: bool) -> Self {
        self.traits = self.traits.with(bits, on);
        self.trait_mask |= bits;
        self
    }
    pub fn bold(self) -> Self {
        self.traits(Traits::BOLD, true)
    }
    pub fn italic(self) -> Self {
        self.traits(Traits::ITALIC, true)
    }
    pub fn mono(self) -> Self {
        self.traits(Traits::MONO, true)
    }
    pub fn condensed(self) -> Self {
        self.traits(Traits::CONDENSED, true)
    }
    pub fn with_cascade(mut self, families: &[&str]) -> Self {
        self.cascade = families.iter().map(|s| s.to_string()).collect();
        self
    }

    /// The set attributes as (key, value) pairs.
    pub fn attributes(&self) -> Vec<(&'static str, AttrValue)> {
        let mut v = Vec::new();
        let mut text = |k: &'static str, o: &Option<String>| {
            if let Some(s) = o {
                v.push((k, AttrValue::Text(s.clone())));
            }
        };
        text("family", &self.family);
        text("style", &self.style);
        text("full-name", &self.full_name);
        text("postscript-name", &self.postscript_name);
        if let Some(s) = self.size {
            v.push(("size", AttrValue::Number(s)));
        }
        if self.trait_mask != 0 {
            v.push(("traits", AttrValue::Traits(self.traits)));
        }
        if let Some(w) = self.weight {
            v.push(("weight", AttrValue::Number(w)));
        }
        if let Some(w) = self.width {
            v.push(("width", AttrValue::Number(w)));
        }
        if let Some(s) = self.slant {
            v.push(("slant", AttrValue::Number(s)));
        }
        if let Some(m) = self.matrix {
            v.push(("matrix", AttrValue::Matrix(m)));
        }
        if !self.variations.is_empty() {
            v.push(("variation", AttrValue::Variations(self.variations.clone())));
        }
        v
    }

    /// Whether the spec has anything set that narrows a match.
    pub fn is_empty(&self) -> bool {
        self.attributes().is_empty() && self.cascade.is_empty()
    }
}

/// How long a registration lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// shipped with the system, cannot be unregistered by apps
    System,
    /// until the process ends
    Process,
    /// until the session ends
    Session,
    /// kept across sessions for the current user
    User,
}

/// Whether a missing font may be fetched or activated on demand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoActivation {
    Default,
    Disabled,
    Enabled,
    Ask,
}

/// What a face is stored in.
#[derive(Clone, Debug)]
pub enum Source {
    Path(String),
    Data(Arc<Vec<u8>>),
}

/// The facts the manager keeps about one face.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceRecord {
    pub family: String,
    pub style: String,
    pub full_name: String,
    pub postscript_name: String,
    pub weight_class: u16,
    pub width_class: u16,
    pub traits: Traits,
    pub italic_angle: f32,
    pub variable: bool,
    pub glyph_count: u16,
}

impl FaceRecord {
    pub fn from_info(i: &FaceInfo) -> Self {
        FaceRecord {
            family: i.family.clone(),
            style: i.style.clone(),
            full_name: i.full_name.clone(),
            postscript_name: i.postscript_name.clone(),
            weight_class: i.weight_class,
            width_class: i.width_class,
            traits: i.traits,
            italic_angle: i.italic_angle,
            variable: i.variable,
            glyph_count: i.glyph_count,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Registered {
    pub record: FaceRecord,
    pub source: Source,
    pub scope: Scope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontError {
    /// the bytes are not a font the renderer can read
    Invalid,
    /// the same face (by PostScript name) is already registered
    AlreadyRegistered,
    NotRegistered,
    /// system fonts cannot be removed
    InUse,
    Io(String),
}

/// A change the manager reports to listeners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontEvent {
    Registered(String),
    Unregistered(String),
}

type Listener = Box<dyn Fn(&FontEvent) + Send>;

/// The registered and installed faces, with matching and change notification.
pub struct FontManager {
    faces: Vec<Registered>,
    generation: u64,
    activation: AutoActivation,
    listeners: Vec<Listener>,
    events: Vec<FontEvent>,
    files: Mutex<HashMap<String, Arc<Vec<u8>>>>,
    collections: Vec<(String, Vec<String>)>,
}

impl Default for FontManager {
    fn default() -> Self {
        FontManager {
            faces: Vec::new(),
            generation: 0,
            activation: AutoActivation::Default,
            listeners: Vec::new(),
            events: Vec::new(),
            files: Mutex::new(HashMap::new()),
            collections: Vec::new(),
        }
    }
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn strip_url(u: &str) -> &str {
    u.strip_prefix("file://").unwrap_or(u)
}

const FONT_EXT: [&str; 4] = ["ttf", "otf", "ttc", "otc"];

impl FontManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the face in `data` for `scope`. Returns the PostScript name.
    pub fn register_data(&mut self, data: Vec<u8>, scope: Scope) -> Result<String, FontError> {
        let info = FaceInfo::parse(&data).ok_or(FontError::Invalid)?;
        self.add(FaceRecord::from_info(&info), Source::Data(Arc::new(data)), scope)
    }

    /// Registers the font file at `url` (a plain path or a `file://` URL).
    pub fn register_url(&mut self, url: &str, scope: Scope) -> Result<String, FontError> {
        let path = strip_url(url);
        let bytes = std::fs::read(path).map_err(|e| FontError::Io(e.to_string()))?;
        let info = FaceInfo::parse(&bytes).ok_or(FontError::Invalid)?;
        let ps = self.add(FaceRecord::from_info(&info), Source::Path(path.to_string()), scope)?;
        if let Ok(mut f) = self.files.lock() {
            f.insert(path.to_string(), Arc::new(bytes));
        }
        Ok(ps)
    }

    fn add(&mut self, record: FaceRecord, source: Source, scope: Scope) -> Result<String, FontError> {
        if self.faces.iter().any(|f| eq_ci(&f.record.postscript_name, &record.postscript_name)) {
            return Err(FontError::AlreadyRegistered);
        }
        let ps = record.postscript_name.clone();
        self.faces.push(Registered { record, source, scope });
        self.changed(FontEvent::Registered(ps.clone()));
        Ok(ps)
    }

    /// Unregisters the face with this PostScript name. System faces stay.
    pub fn unregister(&mut self, postscript_name: &str) -> Result<(), FontError> {
        let i = self.faces.iter().position(|f| eq_ci(&f.record.postscript_name, postscript_name)).ok_or(FontError::NotRegistered)?;
        if self.faces[i].scope == Scope::System {
            return Err(FontError::InUse);
        }
        let gone = self.faces.remove(i);
        if let (Source::Path(p), Ok(mut f)) = (&gone.source, self.files.lock()) {
            f.remove(p);
        }
        self.changed(FontEvent::Unregistered(gone.record.postscript_name));
        Ok(())
    }

    /// Unregisters every face loaded from the file at `url`. Returns how many went.
    pub fn unregister_url(&mut self, url: &str) -> usize {
        let path = strip_url(url).to_string();
        let names: Vec<String> = self
            .faces
            .iter()
            .filter(|f| matches!(&f.source, Source::Path(p) if *p == path) && f.scope != Scope::System)
            .map(|f| f.record.postscript_name.clone())
            .collect();
        names.iter().filter(|n| self.unregister(n).is_ok()).count()
    }

    /// Registers every font file under `dir` (to `depth` levels) as `Scope::System`. Unreadable files are skipped;
    /// returns how many faces were added. Files are read whole, so call this once at startup.
    pub fn scan_dir(&mut self, dir: &std::path::Path, depth: usize) -> usize {
        let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
        let mut n = 0;
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth > 0 {
                    n += self.scan_dir(&p, depth - 1);
                }
            } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| FONT_EXT.iter().any(|x| eq_ci(x, e))) {
                if let Some(s) = p.to_str() {
                    if self.register_url(s, Scope::System).is_ok() {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    fn changed(&mut self, e: FontEvent) {
        self.generation += 1;
        for l in &self.listeners {
            l(&e);
        }
        self.events.push(e);
    }

    /// Calls `f` on every later change.
    pub fn on_change(&mut self, f: impl Fn(&FontEvent) + Send + 'static) {
        self.listeners.push(Box::new(f));
    }

    /// Changes since the last drain, oldest first.
    pub fn drain_events(&mut self) -> Vec<FontEvent> {
        std::mem::take(&mut self.events)
    }

    /// Goes up on every change, for callers that poll instead of listening.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn set_auto_activation(&mut self, a: AutoActivation) {
        self.activation = a;
    }

    pub fn auto_activation(&self) -> AutoActivation {
        self.activation
    }

    pub fn faces(&self) -> &[Registered] {
        &self.faces
    }

    /// Family names, sorted without regard to case, one each.
    pub fn available_families(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for f in &self.faces {
            if !v.iter().any(|x| eq_ci(x, &f.record.family)) {
                v.push(f.record.family.clone());
            }
        }
        v.sort_by_key(|s| s.to_ascii_lowercase());
        v
    }

    /// PostScript names, sorted.
    pub fn available_postscript_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.faces.iter().map(|f| f.record.postscript_name.clone()).collect();
        v.sort_by_key(|s| s.to_ascii_lowercase());
        v
    }

    pub fn faces_in_family(&self, family: &str) -> Vec<&Registered> {
        self.faces.iter().filter(|f| eq_ci(&f.record.family, family)).collect()
    }

    /// The scope a face was registered with.
    pub fn scope_of(&self, postscript_name: &str) -> Option<Scope> {
        self.faces.iter().find(|f| eq_ci(&f.record.postscript_name, postscript_name)).map(|f| f.scope)
    }

    /// The file bytes of face `i`.
    pub fn bytes(&self, i: usize) -> Option<Arc<Vec<u8>>> {
        match &self.faces.get(i)?.source {
            Source::Data(d) => Some(d.clone()),
            Source::Path(p) => {
                if let Some(d) = self.files.lock().ok()?.get(p) {
                    return Some(d.clone());
                }
                let d = Arc::new(std::fs::read(p).ok()?);
                self.files.lock().ok()?.insert(p.clone(), d.clone());
                Some(d)
            }
        }
    }

    fn score(rec: &FaceRecord, spec: &FontSpec) -> f32 {
        let mut s = 0.0;
        if let Some(w) = spec.weight {
            s += (crate::fontinfo::weight_class_to_value(rec.weight_class) - w).abs() * 4.0;
        }
        if let Some(w) = spec.width {
            s += (crate::fontinfo::width_class_to_value(rec.width_class) - w).abs() * 3.0;
        }
        if let Some(sl) = spec.slant {
            s += ((-rec.italic_angle) - sl).abs() / 10.0;
        }
        if spec.trait_mask != 0 {
            let diff = (rec.traits.0 ^ spec.traits.0) & spec.trait_mask;
            s += diff.count_ones() as f32 * 2.0;
        }
        s
    }

    fn passes(rec: &FaceRecord, spec: &FontSpec) -> bool {
        spec.family.as_deref().is_none_or(|n| eq_ci(n, &rec.family))
            && spec.style.as_deref().is_none_or(|n| eq_ci(n, &rec.style))
            && spec.full_name.as_deref().is_none_or(|n| eq_ci(n, &rec.full_name))
            && spec.postscript_name.as_deref().is_none_or(|n| eq_ci(n, &rec.postscript_name))
    }

    /// Indices of the faces that satisfy the spec's names, best first. Ties keep registration order.
    pub fn matching(&self, spec: &FontSpec) -> Vec<usize> {
        let mut v: Vec<(usize, f32)> =
            self.faces.iter().enumerate().filter(|(_, f)| Self::passes(&f.record, spec)).map(|(i, f)| (i, Self::score(&f.record, spec))).collect();
        v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter().map(|x| x.0).collect()
    }

    /// The best face for the spec.
    pub fn best(&self, spec: &FontSpec) -> Option<usize> {
        self.matching(spec).first().copied()
    }

    /// Looks a name up as PostScript name, then full name, then family (best face of it).
    pub fn find_name(&self, name: &str) -> Option<usize> {
        self.best(&FontSpec::new().postscript_name(name))
            .or_else(|| self.best(&FontSpec::new().full_name(name)))
            .or_else(|| self.best(&FontSpec::new().family(name)))
    }

    /// A collection of every face.
    pub fn all_faces(&self) -> FontSet {
        FontSet { name: None, faces: (0..self.faces.len()).collect() }
    }

    /// A collection of the faces that match `spec`.
    pub fn collection_matching(&self, spec: &FontSpec) -> FontSet {
        FontSet { name: None, faces: self.matching(spec) }
    }

    /// A collection of the faces named by the specs (the best face of each, no repeats).
    pub fn collection_from_specs(&self, specs: &[FontSpec]) -> FontSet {
        let mut faces: Vec<usize> = Vec::new();
        for s in specs {
            if let Some(i) = self.best(s) {
                if !faces.contains(&i) {
                    faces.push(i);
                }
            }
        }
        FontSet { name: None, faces }
    }

    /// Stores a named collection of PostScript names (replacing one of the same name).
    pub fn define_collection(&mut self, name: &str, postscript_names: &[&str]) {
        let names: Vec<String> = postscript_names.iter().map(|s| s.to_string()).collect();
        match self.collections.iter_mut().find(|c| eq_ci(&c.0, name)) {
            Some(c) => c.1 = names,
            None => self.collections.push((name.to_string(), names)),
        }
        self.generation += 1;
    }

    /// A named collection with the members that are still registered.
    pub fn named_collection(&self, name: &str) -> Option<FontSet> {
        let c = self.collections.iter().find(|c| eq_ci(&c.0, name))?;
        let faces = c.1.iter().filter_map(|p| self.faces.iter().position(|f| eq_ci(&f.record.postscript_name, p))).collect();
        Some(FontSet { name: Some(c.0.clone()), faces })
    }
}

/// A group of faces, by index into the manager that built it. Valid until faces are unregistered.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSet {
    pub name: Option<String>,
    pub faces: Vec<usize>,
}

impl FontSet {
    /// The faces of the set that also match `spec`.
    pub fn matching(&self, mgr: &FontManager, spec: &FontSpec) -> Vec<usize> {
        mgr.matching(spec).into_iter().filter(|i| self.faces.contains(i)).collect()
    }

    pub fn families(&self, mgr: &FontManager) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for &i in &self.faces {
            if let Some(f) = mgr.faces.get(i) {
                if !v.iter().any(|x| eq_ci(x, &f.record.family)) {
                    v.push(f.record.family.clone());
                }
            }
        }
        v.sort_by_key(|s| s.to_ascii_lowercase());
        v
    }
}

/// A rectangle in text space: origin, then size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// What a glyph override points at.
#[derive(Clone, Debug, PartialEq)]
pub enum GlyphRef {
    Id(u16),
    Name(String),
}

/// A run of text and the face that draws it: `None` is the font itself, `Some(i)` a face of the manager.
#[derive(Clone, Debug, PartialEq)]
pub struct FallbackRun {
    pub face: Option<usize>,
    pub text: String,
}

/// Default fallback families for a language tag such as "en", "ja", "zh-Hant". Ends with symbol and colour emoji
/// families so a missing glyph reaches them before it reaches `.notdef`.
pub fn default_cascade(lang: &str) -> Vec<&'static str> {
    let primary = lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    let mut v: Vec<&'static str> = match primary.as_str() {
        "ja" => vec!["Noto Sans CJK JP", "Noto Sans JP", "IPAexGothic"],
        "ko" => vec!["Noto Sans CJK KR", "Noto Sans KR"],
        "zh" if lang.to_ascii_lowercase().contains("hant") || lang.ends_with("TW") || lang.ends_with("HK") => {
            vec!["Noto Sans CJK TC", "Noto Sans TC"]
        }
        "zh" => vec!["Noto Sans CJK SC", "Noto Sans SC"],
        "ar" | "fa" | "ur" => vec!["Noto Sans Arabic", "Noto Naskh Arabic", "DejaVu Sans"],
        "he" => vec!["Noto Sans Hebrew", "DejaVu Sans"],
        "hi" | "mr" | "ne" => vec!["Noto Sans Devanagari"],
        "th" => vec!["Noto Sans Thai"],
        "ru" | "uk" | "bg" | "sr" | "el" => vec!["Noto Sans", "DejaVu Sans"],
        _ => vec!["Noto Sans", "DejaVu Sans"],
    };
    v.extend(["Noto Sans CJK JP", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Math", "Noto Emoji", "Noto Color Emoji"]);
    let mut seen: Vec<&'static str> = Vec::new();
    v.retain(|n| {
        let fresh = !seen.contains(n);
        seen.push(*n);
        fresh
    });
    v
}

/// One face at one size: names, metrics, glyphs, tables, copies with changed attributes.
#[derive(Clone)]
pub struct FontObject {
    data: Arc<Vec<u8>>,
    info: FaceInfo,
    spec: FontSpec,
    size: f32,
    matrix: [f32; 6],
    variations: Vec<Var>,
    overrides: Vec<(char, GlyphRef)>,
    /// the manager index when it came from one
    face: Option<usize>,
}

impl std::fmt::Debug for FontObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FontObject({} {}pt)", self.info.postscript_name, self.size)
    }
}

impl FontObject {
    /// A font from file bytes. `size` falls back to 12 when not finite and above zero.
    pub fn from_data(data: Vec<u8>, size: f32, matrix: Option<[f32; 6]>) -> Option<FontObject> {
        Self::build(Arc::new(data), None, size, matrix, FontSpec::new())
    }

    fn build(data: Arc<Vec<u8>>, face: Option<usize>, size: f32, matrix: Option<[f32; 6]>, spec: FontSpec) -> Option<FontObject> {
        let info = FaceInfo::parse(&data)?;
        Font::parse(&data)?;
        let size = if size.is_finite() && size > 0.0 { size } else { 12.0 };
        Some(FontObject {
            data,
            info,
            size,
            matrix: matrix.or(spec.matrix).unwrap_or(IDENTITY),
            variations: spec.variations.clone(),
            overrides: Vec::new(),
            face,
            spec,
        })
    }

    /// A font by name (PostScript, full or family) at `size`.
    pub fn from_name(mgr: &FontManager, name: &str, size: f32, matrix: Option<[f32; 6]>) -> Option<FontObject> {
        let i = mgr.find_name(name)?;
        Self::build(mgr.bytes(i)?, Some(i), size, matrix, FontSpec::new())
    }

    /// The best face for `spec`; `size` of the spec (or 12) applies.
    pub fn from_spec(mgr: &FontManager, spec: &FontSpec) -> Option<FontObject> {
        let i = mgr.best(spec)?;
        Self::build(mgr.bytes(i)?, Some(i), spec.size.unwrap_or(12.0), spec.matrix, spec.clone())
    }

    pub fn size(&self) -> f32 {
        self.size
    }
    pub fn matrix(&self) -> [f32; 6] {
        self.matrix
    }
    pub fn face_index(&self) -> Option<usize> {
        self.face
    }

    /// The same face at another size.
    pub fn with_size(&self, size: f32) -> FontObject {
        let mut f = self.clone();
        if size.is_finite() && size > 0.0 {
            f.size = size;
        }
        f
    }

    pub fn with_matrix(&self, m: [f32; 6]) -> FontObject {
        let mut f = self.clone();
        f.matrix = m;
        f
    }

    /// The same face with variation axis values (replacing earlier values of the same axis).
    pub fn with_variations(&self, vars: &[Var]) -> FontObject {
        let mut f = self.clone();
        for v in vars {
            f.variations.retain(|x| x.0 != v.0);
            f.variations.push(*v);
        }
        f.spec.variations = f.variations.clone();
        f
    }

    /// A copy changed per `spec`: names and traits re-match inside the current family unless the spec names another.
    /// Returns None when nothing matches.
    pub fn with_spec(&self, mgr: &FontManager, spec: &FontSpec) -> Option<FontObject> {
        let mut want = spec.clone();
        if want.family.is_none() && want.postscript_name.is_none() && want.full_name.is_none() {
            want.family = Some(self.info.family.clone());
        }
        let i = mgr.best(&want)?;
        let mut f = Self::build(mgr.bytes(i)?, Some(i), spec.size.unwrap_or(self.size), spec.matrix.or(Some(self.matrix)), want)?;
        for v in &self.variations {
            if !f.variations.iter().any(|x| x.0 == v.0) {
                f.variations.push(*v);
            }
        }
        f.overrides = self.overrides.clone();
        Some(f)
    }

    /// A copy in another family, keeping the traits of this face.
    pub fn with_family(&self, mgr: &FontManager, family: &str) -> Option<FontObject> {
        let mut spec = FontSpec::new().family(family);
        spec.traits = self.info.traits;
        spec.trait_mask = Traits::BOLD | Traits::ITALIC | Traits::MONO | Traits::CONDENSED;
        spec.weight = Some(self.info.weight_value());
        self.with_spec(mgr, &spec)
    }

    /// A copy with the trait bits in `mask` set to `bits`, picking the family's best matching face.
    pub fn with_traits(&self, mgr: &FontManager, bits: u32, mask: u32) -> Option<FontObject> {
        let mut spec = FontSpec::new().family(&self.info.family);
        spec.traits = Traits((self.info.traits.0 & !mask) | (bits & mask));
        spec.trait_mask = Traits::BOLD | Traits::ITALIC | Traits::MONO | Traits::CONDENSED | Traits::EXPANDED;
        spec.weight = Some(if spec.traits.is_bold() { self.info.weight_value().max(0.4) } else { self.info.weight_value().min(0.0) });
        self.with_spec(mgr, &spec)
    }

    // names

    pub fn family_name(&self) -> &str {
        &self.info.family
    }
    pub fn style_name(&self) -> &str {
        &self.info.style
    }
    pub fn full_name(&self) -> &str {
        &self.info.full_name
    }
    pub fn postscript_name(&self) -> &str {
        &self.info.postscript_name
    }
    /// The name a user sees: the full name.
    pub fn display_name(&self) -> &str {
        &self.info.full_name
    }
    /// The name of any `name` table entry, for callers that need localised or extra names.
    pub fn name_record(&self, id: NameId) -> Option<String> {
        Tables::new(&self.data)?.name(id)
    }
    pub fn symbolic_traits(&self) -> Traits {
        self.info.traits
    }

    /// The descriptor of this font with every known attribute filled in.
    pub fn descriptor(&self) -> FontSpec {
        let mut s = self.spec.clone();
        s.family = Some(self.info.family.clone());
        s.style = Some(self.info.style.clone());
        s.full_name = Some(self.info.full_name.clone());
        s.postscript_name = Some(self.info.postscript_name.clone());
        s.size = Some(self.size);
        s.traits = self.info.traits;
        s.trait_mask = Traits::BOLD | Traits::ITALIC | Traits::MONO | Traits::CONDENSED | Traits::EXPANDED;
        s.weight = Some(self.info.weight_value());
        s.width = Some(self.info.width_value());
        s.slant = Some(-self.info.italic_angle);
        s.matrix = (self.matrix != IDENTITY).then_some(self.matrix);
        s.variations = self.variations.clone();
        s
    }

    /// The weight class this font is closest to.
    pub fn weight_class(&self) -> u16 {
        self.info.weight_class
    }

    // metrics, all in points at this size

    fn k(&self) -> f32 {
        self.size / self.info.units_per_em as f32
    }
    pub fn units_per_em(&self) -> u16 {
        self.info.units_per_em
    }
    pub fn ascent(&self) -> f32 {
        self.info.ascent as f32 * self.k()
    }
    /// Positive distance below the baseline.
    pub fn descent(&self) -> f32 {
        -(self.info.descent as f32) * self.k()
    }
    pub fn leading(&self) -> f32 {
        self.info.line_gap as f32 * self.k()
    }
    pub fn cap_height(&self) -> f32 {
        self.info.cap_height as f32 * self.k()
    }
    pub fn x_height(&self) -> f32 {
        self.info.x_height as f32 * self.k()
    }
    pub fn underline_position(&self) -> f32 {
        self.info.underline_position as f32 * self.k()
    }
    pub fn underline_thickness(&self) -> f32 {
        self.info.underline_thickness as f32 * self.k()
    }
    /// Degrees clockwise from upright (positive leans right).
    pub fn slant_angle(&self) -> f32 {
        -self.info.italic_angle
    }
    pub fn glyph_count(&self) -> u16 {
        self.info.glyph_count
    }
    /// The union of all glyph boxes.
    pub fn bounding_box(&self) -> RectF {
        let b = self.info.bbox;
        let k = self.k();
        RectF { x: b[0] as f32 * k, y: b[1] as f32 * k, w: (b[2] as f32 - b[0] as f32) * k, h: (b[3] as f32 - b[1] as f32) * k }
    }
    /// Ascent plus descent plus leading.
    pub fn line_height(&self) -> f32 {
        self.ascent() + self.descent() + self.leading()
    }

    // glyphs

    fn font(&self) -> Option<Font<'_>> {
        Font::parse(&self.data)
    }

    /// The glyph for `ch`, honouring overrides; 0 when the face has none.
    pub fn glyph_for_char(&self, ch: char) -> u16 {
        if let Some((_, r)) = self.overrides.iter().find(|o| o.0 == ch) {
            match r {
                GlyphRef::Id(g) => return *g,
                GlyphRef::Name(n) => {
                    if let Some(g) = self.glyph_named(n) {
                        return g;
                    }
                }
            }
        }
        self.font().map_or(0, |f| f.glyph(ch))
    }

    pub fn glyphs_for_str(&self, s: &str) -> Vec<u16> {
        s.chars().map(|c| self.glyph_for_char(c)).collect()
    }

    /// Replaces the glyph drawn for `ch` (a glyph id or a glyph name). Name overrides that the face cannot
    /// resolve are ignored at lookup. Returns the changed copy.
    pub fn with_glyph_override(&self, ch: char, r: GlyphRef) -> FontObject {
        let mut f = self.clone();
        f.overrides.retain(|o| o.0 != ch);
        f.overrides.push((ch, r));
        f
    }

    pub fn glyph_name(&self, gid: u16) -> String {
        GlyphTables::new(&self.data).map(|g| g.glyph_name(gid)).unwrap_or_else(|| ".notdef".to_string())
    }

    /// The glyph with this name: post names, else CFF names.
    pub fn glyph_named(&self, name: &str) -> Option<u16> {
        GlyphTables::new(&self.data).and_then(|g| g.glyph_named(name)).or_else(|| self.font()?.glyph_named(name))
    }

    /// Advance widths in points.
    pub fn advances(&self, gids: &[u16]) -> Vec<f32> {
        let g = GlyphTables::new(&self.data);
        let k = self.k();
        gids.iter().map(|&id| g.as_ref().map_or(0.0, |g| g.advance(id) as f32 * k)).collect()
    }

    /// Width of `text` in points, with variations applied.
    pub fn text_width(&self, text: &str) -> f32 {
        self.font().map_or(0.0, |f| f.measure_var(text, self.size, &self.variations))
    }

    /// Ink boxes in points relative to each glyph origin, y up. Empty (zero) for glyphs the face has no box for
    /// (outline-less formats).
    pub fn bounding_rects(&self, gids: &[u16]) -> Vec<RectF> {
        let g = GlyphTables::new(&self.data);
        let k = self.k();
        gids.iter()
            .map(|&id| match g.as_ref().and_then(|g| g.glyph_box(id)) {
                Some(b) => RectF {
                    x: b.x_min as f32 * k,
                    y: b.y_min as f32 * k,
                    w: (b.x_max as f32 - b.x_min as f32) * k,
                    h: (b.y_max as f32 - b.y_min as f32) * k,
                },
                None => RectF::default(),
            })
            .collect()
    }

    /// Left and right optical bounds in points: how far the ink sits inside the advance box on each side.
    pub fn optical_bounds(&self, gids: &[u16]) -> Vec<(f32, f32)> {
        let g = GlyphTables::new(&self.data);
        let k = self.k();
        gids.iter()
            .map(|&id| g.as_ref().and_then(|g| g.optical_bounds(id)).map_or((0.0, 0.0), |(l, r)| (l as f32 * k, r as f32 * k)))
            .collect()
    }

    // tables

    pub fn available_tables(&self) -> Vec<[u8; 4]> {
        Tables::new(&self.data).map(|t| t.tags()).unwrap_or_default()
    }

    pub fn copy_table(&self, tag: &[u8; 4]) -> Option<Vec<u8>> {
        Tables::new(&self.data)?.table(tag).map(|t| t.to_vec())
    }

    // variation axes

    /// (tag, min, default, max) for each axis; empty for static faces.
    pub fn variation_axes(&self) -> Vec<([u8; 4], f32, f32, f32)> {
        self.font().map(|f| f.axes().iter().map(|a| (a.tag, a.min, a.default, a.max)).collect()).unwrap_or_default()
    }

    pub fn variations(&self) -> &[Var] {
        &self.variations
    }

    // fallback and drawing

    /// Splits `text` into runs: characters this face covers stay with it, others go to the first cascade face
    /// that covers them (the spec's cascade, then the language default, then every other face). Characters
    /// nothing covers stay with this face.
    pub fn fallback_runs(&self, mgr: &FontManager, text: &str, lang: &str) -> Vec<FallbackRun> {
        let mut order: Vec<usize> = Vec::new();
        let mut names: Vec<String> = self.spec.cascade.clone();
        names.extend(default_cascade(lang).into_iter().map(String::from));
        for n in &names {
            for i in mgr.matching(&FontSpec::new().family(n)) {
                if !order.contains(&i) && Some(i) != self.face {
                    order.push(i);
                }
            }
        }
        for i in 0..mgr.faces().len() {
            if !order.contains(&i) && Some(i) != self.face {
                order.push(i);
            }
        }
        let mut loaded: HashMap<usize, Arc<Vec<u8>>> = HashMap::new();
        let mut runs: Vec<FallbackRun> = Vec::new();
        for ch in text.chars() {
            let mut face = None;
            if self.glyph_for_char(ch) == 0 && !ch.is_whitespace() && !ch.is_control() {
                for &i in &order {
                    let bytes = match loaded.get(&i) {
                        Some(b) => b.clone(),
                        None => match mgr.bytes(i) {
                            Some(b) => {
                                loaded.insert(i, b.clone());
                                b
                            }
                            None => continue,
                        },
                    };
                    if Font::parse(&bytes).is_some_and(|f| f.glyph(ch) != 0) {
                        face = Some(i);
                        break;
                    }
                }
            }
            match runs.last_mut() {
                Some(r) if r.face == face => r.text.push(ch),
                _ => runs.push(FallbackRun { face, text: ch.to_string() }),
            }
        }
        runs
    }

    /// Draws `text` as images with their x offsets in px, cascading to other faces for missing glyphs.
    pub fn render_with_fallback(&self, mgr: &FontManager, text: &str, lang: &str, color: u32) -> Vec<(f32, Image)> {
        let mut out = Vec::new();
        let mut x = 0.0;
        let embolden = if self.spec.traits.is_bold() && !self.info.traits.is_bold() { self.size * 0.04 } else { 0.0 };
        for run in self.fallback_runs(mgr, text, lang) {
            let st = Style { px: self.size, embolden, color, shadow: false };
            let (img, w) = match run.face {
                None => match self.font() {
                    Some(f) => (f.render_var(&run.text, &st, &self.variations), f.measure_var(&run.text, self.size, &self.variations)),
                    None => continue,
                },
                Some(i) => {
                    let Some(bytes) = mgr.bytes(i) else { continue };
                    let Some(f) = Font::parse(&bytes) else { continue };
                    (f.render(&run.text, &st), f.measure(&run.text, self.size))
                }
            };
            out.push((x, img));
            x += w;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).expect("assets/SFUI.ttf")
    }

    fn mgr() -> FontManager {
        let mut m = FontManager::new();
        m.register_data(sf(), Scope::Process).unwrap();
        m
    }

    #[test]
    fn spec_builder_and_attributes() {
        let s = FontSpec::new().family("Foo").size(14.0).bold().weight(2.0).variation(b"wght", 500.0);
        assert_eq!(s.weight, Some(1.0));
        assert!(s.traits.is_bold() && s.trait_mask == Traits::BOLD);
        let a = s.attributes();
        assert!(a.iter().any(|(k, _)| *k == "family") && a.iter().any(|(k, _)| *k == "variation"));
        assert!(FontSpec::new().is_empty() && !s.is_empty());
        assert_eq!(FontSpec::new().size(-1.0).size, None);
        assert_eq!(FontSpec::new().variation(b"wght", 1.0).variation(b"wght", 2.0).variations.len(), 1);
    }

    #[test]
    fn manager_registers_lists_and_notifies() {
        let mut m = FontManager::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        m.on_change(move |e| s2.lock().unwrap().push(e.clone()));
        assert_eq!(m.register_data(vec![1, 2, 3], Scope::Process), Err(FontError::Invalid));
        let ps = m.register_data(sf(), Scope::Process).unwrap();
        assert_eq!(m.register_data(sf(), Scope::Process), Err(FontError::AlreadyRegistered));
        assert_eq!(m.available_postscript_names(), vec![ps.clone()]);
        assert_eq!(m.available_families().len(), 1);
        assert_eq!(m.scope_of(&ps), Some(Scope::Process));
        assert_eq!(m.generation(), 1);
        assert!(m.unregister(&ps).is_ok());
        assert_eq!(m.unregister(&ps), Err(FontError::NotRegistered));
        assert_eq!(m.drain_events(), vec![FontEvent::Registered(ps.clone()), FontEvent::Unregistered(ps.clone())]);
        assert!(m.drain_events().is_empty());
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(m.generation(), 2);
    }

    #[test]
    fn system_faces_stay() {
        let mut m = FontManager::new();
        let ps = m.register_data(sf(), Scope::System).unwrap();
        assert_eq!(m.unregister(&ps), Err(FontError::InUse));
        assert_eq!(m.unregister_url("/nowhere"), 0);
    }

    #[test]
    fn auto_activation_setting() {
        let mut m = FontManager::new();
        assert_eq!(m.auto_activation(), AutoActivation::Default);
        m.set_auto_activation(AutoActivation::Disabled);
        assert_eq!(m.auto_activation(), AutoActivation::Disabled);
    }

    #[test]
    fn url_registration_reads_a_file_and_unregisters_by_url() {
        let dir = std::env::temp_dir().join(format!("fontobj-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.ttf");
        std::fs::write(&p, sf()).unwrap();
        let mut m = FontManager::new();
        let url = format!("file://{}", p.display());
        m.register_url(&url, Scope::User).unwrap();
        assert!(m.bytes(0).is_some());
        assert_eq!(m.unregister_url(&url), 1);
        assert!(m.faces().is_empty());
        assert!(matches!(m.register_url("/no/such/file.ttf", Scope::User), Err(FontError::Io(_))));
        let mut m2 = FontManager::new();
        assert_eq!(m2.scan_dir(&dir, 1), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn matching_by_names_and_rank() {
        let m = mgr();
        let fam = m.faces()[0].record.family.clone();
        assert_eq!(m.matching(&FontSpec::new().family(&fam.to_uppercase())), vec![0]);
        assert!(m.matching(&FontSpec::new().family("No Such Family")).is_empty());
        assert_eq!(m.find_name(&m.faces()[0].record.full_name), Some(0));
        assert_eq!(m.best(&FontSpec::new().bold().weight(0.8)), Some(0));
        let set = m.all_faces();
        assert_eq!(set.matching(&m, &FontSpec::new()), vec![0]);
        assert_eq!(set.families(&m), vec![fam]);
        assert_eq!(m.collection_from_specs(&[FontSpec::new(), FontSpec::new()]).faces, vec![0]);
        assert!(m.collection_matching(&FontSpec::new().family("zz")).faces.is_empty());
    }

    #[test]
    fn named_collections_follow_registration() {
        let mut m = mgr();
        let ps = m.faces()[0].record.postscript_name.clone();
        m.define_collection("Mine", &[&ps, "Gone-Regular"]);
        assert_eq!(m.named_collection("mine").unwrap().faces, vec![0]);
        assert!(m.named_collection("other").is_none());
    }

    #[test]
    fn font_object_metrics_scale_with_size() {
        let m = mgr();
        let a = FontObject::from_name(&m, &m.faces()[0].record.postscript_name, 10.0, None).unwrap();
        let b = a.with_size(20.0);
        assert!((b.ascent() - a.ascent() * 2.0).abs() < 1e-3);
        assert!(a.ascent() > 0.0 && a.descent() > 0.0);
        assert!(a.cap_height() >= a.x_height() && a.x_height() >= 0.0);
        assert!(a.bounding_box().w > 0.0 && a.glyph_count() > 10);
        assert!(a.line_height() >= a.ascent() + a.descent());
        assert_eq!(a.with_size(f32::NAN).size(), 10.0);
        assert_eq!(a.matrix(), IDENTITY);
        assert_eq!(FontObject::from_name(&m, "zzz", 10.0, None).is_none(), true);
    }

    #[test]
    fn font_object_names_and_descriptor() {
        let m = mgr();
        let f = FontObject::from_spec(&m, &FontSpec::new().size(15.0)).unwrap();
        assert_eq!(f.size(), 15.0);
        assert!(!f.family_name().is_empty() && !f.postscript_name().is_empty());
        assert_eq!(f.display_name(), f.full_name());
        let d = f.descriptor();
        assert_eq!(d.postscript_name.as_deref(), Some(f.postscript_name()));
        assert_eq!(d.size, Some(15.0));
        assert!(f.name_record(NameId::Family).is_some());
        let again = FontObject::from_spec(&m, &d).unwrap();
        assert_eq!(again.postscript_name(), f.postscript_name());
    }

    #[test]
    fn copies_keep_or_change_attributes() {
        let m = mgr();
        let f = FontObject::from_spec(&m, &FontSpec::new().size(12.0)).unwrap();
        let g = f.with_traits(&m, Traits::BOLD, Traits::BOLD).unwrap();
        assert_eq!(g.size(), 12.0);
        let h = f.with_family(&m, f.family_name()).unwrap();
        assert_eq!(h.postscript_name(), f.postscript_name());
        assert!(f.with_family(&m, "Nope").is_none());
        let v = f.with_variations(&[(*b"wght", 700.0)]).with_variations(&[(*b"wght", 300.0)]);
        assert_eq!(v.variations(), &[(*b"wght", 300.0)]);
        assert_eq!(f.with_matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]).matrix()[0], 2.0);
    }

    #[test]
    fn glyph_lookup_overrides_and_names() {
        let m = mgr();
        let f = FontObject::from_spec(&m, &FontSpec::new().size(16.0)).unwrap();
        let ids = f.glyphs_for_str("Hi");
        assert!(ids.iter().all(|&g| g != 0));
        let w = f.advances(&ids);
        assert!(w.iter().all(|&x| x > 0.0));
        assert!((f.text_width("Hi") - w.iter().sum::<f32>()).abs() < 0.5);
        let o = f.with_glyph_override('H', GlyphRef::Id(ids[1]));
        assert_eq!(o.glyph_for_char('H'), ids[1]);
        assert_eq!(f.glyph_for_char('H'), ids[0]);
        let unresolved = f.with_glyph_override('H', GlyphRef::Name("no-such-glyph-name".into()));
        assert_eq!(unresolved.glyph_for_char('H'), ids[0]);
        assert_eq!(f.glyph_name(0), ".notdef");
        let r = f.bounding_rects(&ids);
        assert!(r[0].w > 0.0 && r[0].h > 0.0);
        let ob = f.optical_bounds(&ids);
        assert_eq!(ob.len(), 2);
        assert!(f.bounding_rects(&[u16::MAX])[0] == RectF::default());
    }

    #[test]
    fn tables_and_axes() {
        let m = mgr();
        let f = FontObject::from_spec(&m, &FontSpec::new()).unwrap();
        assert!(f.available_tables().contains(b"head"));
        assert_eq!(f.copy_table(b"head").unwrap().len() >= 54, true);
        assert!(f.copy_table(b"ZZZZ").is_none());
        for (_, lo, def, hi) in f.variation_axes() {
            assert!(lo <= def && def <= hi);
        }
        assert!(FontObject::from_data(vec![0; 8], 12.0, None).is_none());
    }

    #[test]
    fn cascade_defaults_end_with_emoji_and_symbols() {
        for l in ["en", "ja", "zh-Hant", "zh-Hans", "ar", "xx"] {
            let c = default_cascade(l);
            assert!(c.contains(&"Noto Color Emoji") && c.contains(&"Noto Sans Symbols"), "{l}");
            let mut d = c.clone();
            d.sort();
            d.dedup();
            assert_eq!(d.len(), c.len(), "{l} repeats");
        }
        assert_eq!(default_cascade("ja")[0], "Noto Sans CJK JP");
        assert_eq!(default_cascade("zh-Hant")[0], "Noto Sans CJK TC");
        assert_eq!(default_cascade("zh_CN")[0], "Noto Sans CJK SC");
    }

    #[test]
    fn fallback_keeps_covered_text_on_the_font() {
        let m = mgr();
        let f = FontObject::from_spec(&m, &FontSpec::new().size(14.0)).unwrap();
        let runs = f.fallback_runs(&m, "Hello world", "en");
        assert_eq!(runs, vec![FallbackRun { face: None, text: "Hello world".into() }]);
        // a character nothing covers stays on the font
        let runs = f.fallback_runs(&m, "a\u{10FFFE}b", "en");
        assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<String>(), "a\u{10FFFE}b");
        assert!(runs.iter().all(|r| r.face.is_none()));
        let imgs = f.render_with_fallback(&m, "Hello", "en", 0xffffffff);
        assert_eq!(imgs.len(), 1);
        assert!(imgs[0].1.w > 0);
    }

    #[test]
    fn fallback_uses_another_face_that_has_the_glyph() {
        let mut m = FontManager::new();
        m.register_data(sf(), Scope::Process).unwrap();
        // find a character the bundled face lacks but some other bundled asset covers
        let assets = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets");
        let Ok(rd) = std::fs::read_dir(assets) else { return };
        let primary = FontObject::from_spec(&m, &FontSpec::new()).unwrap();
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("ttf") || p.file_name().is_some_and(|n| n == "SFUI.ttf") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            let Some(font) = Font::parse(&bytes) else { continue };
            let Some(ch) = (0x250u32..0x3000).filter_map(char::from_u32).find(|&c| font.glyph(c) != 0 && primary.glyph_for_char(c) == 0) else { continue };
            if m.register_data(bytes, Scope::Process).is_err() {
                continue;
            }
            let runs = primary.fallback_runs(&m, &format!("a{ch}b"), "en");
            assert!(runs.iter().any(|r| r.face.is_some() && r.text.contains(ch)));
            return;
        }
    }
}
