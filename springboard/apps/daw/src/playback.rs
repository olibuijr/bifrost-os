//! Playback through the shell's audio service. The service plays files, so the song is rendered offline
//! to a cached WAV (keyed by a hash of the project) and queued as one track. Transport state comes back
//! from the service snapshot.

use crate::model::Project;
use crate::render::{self, RenderOpts};
use crate::wav;
use bifrost_app::audio::{Client, Track};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub struct Player {
    client: Client,
    cache: PathBuf,
    plugins: PathBuf,
    /// Hash of the project that is currently queued (re-render only when it changes).
    queued: Option<u64>,
    next_id: u64,
}

impl Player {
    pub fn new(root: &Path, cache: PathBuf, plugins: PathBuf) -> Self {
        Player { client: Client::for_app(root), cache, plugins, queued: None, next_id: 1 }
    }

    /// Drive the service connection; call once per loop turn.
    pub fn tick(&mut self, now: f64) {
        self.client.pump();
        self.client.tick(now);
    }

    pub fn playing(&self) -> bool {
        self.client.snapshot().playing
    }

    pub fn position_ms(&self) -> f64 {
        self.client.snapshot().position_ms
    }

    pub fn connected(&self) -> bool {
        self.client.connected()
    }

    /// Renders the project (once per distinct content) and starts it from the beginning.
    pub fn play(&mut self, project: &Project) -> Result<PathBuf, String> {
        let key = hash_of(project);
        let path = self.cache.join(format!("{}-{key:016x}.wav", sanitize(&project.id)));
        if !path.exists() {
            std::fs::create_dir_all(&self.cache).map_err(|e| e.to_string())?;
            let opts = RenderOpts { plugins: self.plugins.clone(), ..Default::default() };
            let rendered = render::render(project, &opts);
            let bytes = wav::encode16(&rendered.left, &rendered.right, rendered.sample_rate, opts.seed);
            wav::write_file(&path, &bytes).map_err(|e| e.to_string())?;
        }
        if self.queued != Some(key) {
            self.next_id += 1;
            let track = Track { id: self.next_id, path: path.to_string_lossy().into_owned(), url: None };
            if !self.client.play_queue(vec![track], 0, false, 0) {
                return Err("audio service unavailable".into());
            }
            self.queued = Some(key);
        } else if !self.client.play() {
            return Err("audio service unavailable".into());
        }
        Ok(path)
    }

    pub fn pause(&mut self) {
        self.client.pause();
    }

    pub fn stop(&mut self) {
        self.client.stop();
        self.queued = None;
    }
}

fn hash_of(project: &Project) -> u64 {
    let mut h = DefaultHasher::new();
    project.to_json().hash(&mut h);
    h.finish()
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}
