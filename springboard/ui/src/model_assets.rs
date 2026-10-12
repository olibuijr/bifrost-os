//! Models that arrive at runtime, tracked as named assets. An asset moves through a download, a compile and ready:
//! [`AssetCatalog::begin_download`] names the target version and its byte total, [`AssetCatalog::progress`] reports
//! bytes, [`AssetCatalog::finish_download`] takes a digest of the bytes, then [`AssetCatalog::begin_compile`] and
//! [`AssetCatalog::finish_compile`]. A failure can be reported while a version is in flight.
//! The ready version and the target version are kept apart: a newer version that is downloading, compiling or failed
//! leaves the ready version usable, and a version that is not newer than the ready one is refused.
//! This is a pure state machine. The caller reports what happened; nothing here fetches, compiles or reads a file.
use std::collections::BTreeMap;

/// Where an asset's target version is in its life cycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// `received` of `total` bytes of the target version have arrived.
    Downloading { received: u64, total: u64 },
    /// Every byte of the target version arrived. `digest` identifies the bytes.
    Downloaded { digest: u64 },
    /// The target version is being compiled on this device.
    Compiling,
    /// The target version is compiled, so it is the ready version.
    Ready,
    /// The last attempt failed for `reason`. The ready version, if any, is still usable.
    Failed { reason: String },
}

/// Why a step on an asset was refused. The state is unchanged after an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// No asset has this name.
    NotFound,
    /// A version is already downloading, downloaded or compiling.
    Busy,
    /// The requested version is not newer than the ready one, which is `ready`.
    NotNewer { ready: u32 },
    /// A progress report is larger than the byte total.
    Progress,
    /// The download has not received every byte yet.
    NotFinished,
    /// The step does not apply in the asset's current phase. `phase` and `action` are the names of both.
    Transition { phase: &'static str, action: &'static str },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Asset {
    target: u32,
    ready: Option<u32>,
    phase: Phase,
}

fn phase_name(phase: &Phase) -> &'static str {
    match phase {
        Phase::Downloading { .. } => "downloading",
        Phase::Downloaded { .. } => "downloaded",
        Phase::Compiling => "compiling",
        Phase::Ready => "ready",
        Phase::Failed { .. } => "failed",
    }
}

/// Every model asset of the app, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetCatalog {
    assets: BTreeMap<String, Asset>,
}

impl AssetCatalog {
    /// An empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts downloading `version` of an asset that is `total` bytes long. A new asset is created on first use.
    /// Refused while a version is in flight, and when `version` is not newer than the ready version. A failed
    /// attempt may be retried with the same version when no version is ready.
    pub fn begin_download(&mut self, name: &str, version: u32, total: u64) -> Result<(), AssetError> {
        if !self.assets.contains_key(name) {
            let asset = Asset { target: version, ready: None, phase: Phase::Downloading { received: 0, total } };
            self.assets.insert(name.to_owned(), asset);
            return Ok(());
        }
        let asset = self.assets.get_mut(name).ok_or(AssetError::NotFound)?;
        if matches!(asset.phase, Phase::Downloading { .. } | Phase::Downloaded { .. } | Phase::Compiling) {
            return Err(AssetError::Busy);
        }
        if let Some(ready) = asset.ready {
            if version <= ready {
                return Err(AssetError::NotNewer { ready });
            }
        }
        asset.target = version;
        asset.phase = Phase::Downloading { received: 0, total };
        Ok(())
    }

    /// Records how many bytes of the target version have arrived. Must not pass the byte total.
    pub fn progress(&mut self, name: &str, received: u64) -> Result<(), AssetError> {
        let asset = self.assets.get_mut(name).ok_or(AssetError::NotFound)?;
        let total = match asset.phase {
            Phase::Downloading { total, .. } => total,
            _ => return Err(Self::transition(asset, "progress")),
        };
        if received > total {
            return Err(AssetError::Progress);
        }
        asset.phase = Phase::Downloading { received, total };
        Ok(())
    }

    /// Completes the download once every byte has arrived, and records the digest of those bytes.
    pub fn finish_download(&mut self, name: &str, digest: u64) -> Result<(), AssetError> {
        let asset = self.assets.get_mut(name).ok_or(AssetError::NotFound)?;
        let (received, total) = match asset.phase {
            Phase::Downloading { received, total } => (received, total),
            _ => return Err(Self::transition(asset, "finish download")),
        };
        if received != total {
            return Err(AssetError::NotFinished);
        }
        asset.phase = Phase::Downloaded { digest };
        Ok(())
    }

    /// Starts compiling the downloaded version.
    pub fn begin_compile(&mut self, name: &str) -> Result<(), AssetError> {
        let asset = self.assets.get_mut(name).ok_or(AssetError::NotFound)?;
        if !matches!(asset.phase, Phase::Downloaded { .. }) {
            return Err(Self::transition(asset, "compile"));
        }
        asset.phase = Phase::Compiling;
        Ok(())
    }

    /// Completes the compile. The target version becomes the ready version.
    pub fn finish_compile(&mut self, name: &str) -> Result<(), AssetError> {
        let asset = self.assets.get_mut(name).ok_or(AssetError::NotFound)?;
        if !matches!(asset.phase, Phase::Compiling) {
            return Err(Self::transition(asset, "finish compile"));
        }
        asset.ready = Some(asset.target);
        asset.phase = Phase::Ready;
        Ok(())
    }

    /// Records that the attempt failed. Allowed while a version is downloading, downloaded or compiling.
    pub fn fail(&mut self, name: &str, reason: &str) -> Result<(), AssetError> {
        let asset = self.assets.get_mut(name).ok_or(AssetError::NotFound)?;
        if matches!(asset.phase, Phase::Ready | Phase::Failed { .. }) {
            return Err(Self::transition(asset, "fail"));
        }
        asset.phase = Phase::Failed { reason: reason.to_owned() };
        Ok(())
    }

    /// Removes an asset. Returns whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.assets.remove(name).is_some()
    }

    /// The phase of the target version, or `None` for an unknown name.
    pub fn phase(&self, name: &str) -> Option<&Phase> {
        self.assets.get(name).map(|asset| &asset.phase)
    }

    /// The version that may be used now, if one has been compiled.
    pub fn ready_version(&self, name: &str) -> Option<u32> {
        self.assets.get(name).and_then(|asset| asset.ready)
    }

    /// The version being downloaded, compiled or failed, or the ready version when nothing is in flight.
    pub fn target_version(&self, name: &str) -> Option<u32> {
        self.assets.get(name).map(|asset| asset.target)
    }

    /// The asset names in order.
    pub fn names(&self) -> Vec<&str> {
        self.assets.keys().map(|name| name.as_str()).collect()
    }

    fn transition(asset: &Asset, action: &'static str) -> AssetError {
        AssetError::Transition { phase: phase_name(&asset.phase), action }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Brings `name` to ready at `version` with a 10-byte download.
    fn ready_at(c: &mut AssetCatalog, name: &str, version: u32) {
        c.begin_download(name, version, 10).unwrap();
        c.progress(name, 10).unwrap();
        c.finish_download(name, 7).unwrap();
        c.begin_compile(name).unwrap();
        c.finish_compile(name).unwrap();
    }

    #[test]
    fn full_cycle_makes_the_version_ready() {
        let mut c = AssetCatalog::new();
        c.begin_download("tagger", 1, 100).unwrap();
        assert_eq!(c.phase("tagger"), Some(&Phase::Downloading { received: 0, total: 100 }));
        c.progress("tagger", 60).unwrap();
        c.progress("tagger", 100).unwrap();
        c.finish_download("tagger", 7).unwrap();
        assert_eq!(c.phase("tagger"), Some(&Phase::Downloaded { digest: 7 }));
        c.begin_compile("tagger").unwrap();
        c.finish_compile("tagger").unwrap();
        assert_eq!(c.phase("tagger"), Some(&Phase::Ready));
        assert_eq!((c.ready_version("tagger"), c.target_version("tagger")), (Some(1), Some(1)));
    }

    #[test]
    fn progress_and_finish_check_the_byte_count() {
        let mut c = AssetCatalog::new();
        c.begin_download("a", 1, 100).unwrap();
        assert_eq!(c.progress("a", 101), Err(AssetError::Progress));
        c.progress("a", 40).unwrap();
        assert_eq!(c.finish_download("a", 1), Err(AssetError::NotFinished));
        assert_eq!(c.phase("a"), Some(&Phase::Downloading { received: 40, total: 100 }));
        assert_eq!(c.progress("missing", 1), Err(AssetError::NotFound));
    }

    #[test]
    fn steps_out_of_order_are_refused_and_change_nothing() {
        let mut c = AssetCatalog::new();
        c.begin_download("a", 1, 10).unwrap();
        assert_eq!(c.begin_compile("a"), Err(AssetError::Transition { phase: "downloading", action: "compile" }));
        assert_eq!(c.finish_compile("a"), Err(AssetError::Transition { phase: "downloading", action: "finish compile" }));
        assert_eq!(c.phase("a"), Some(&Phase::Downloading { received: 0, total: 10 }));
    }

    #[test]
    fn an_update_keeps_the_ready_version_usable_until_it_is_ready() {
        let mut c = AssetCatalog::new();
        ready_at(&mut c, "a", 1);
        c.begin_download("a", 2, 50).unwrap();
        assert_eq!((c.ready_version("a"), c.target_version("a")), (Some(1), Some(2)));
        c.fail("a", "offline").unwrap();
        assert_eq!(c.phase("a"), Some(&Phase::Failed { reason: "offline".to_owned() }));
        assert_eq!(c.ready_version("a"), Some(1));
        // Not newer than the ready version: refused. The failed version 2 may be retried.
        assert_eq!(c.begin_download("a", 1, 50), Err(AssetError::NotNewer { ready: 1 }));
        c.begin_download("a", 2, 50).unwrap();
        assert_eq!(c.phase("a"), Some(&Phase::Downloading { received: 0, total: 50 }));
    }

    #[test]
    fn a_second_version_is_refused_while_one_is_in_flight() {
        let mut c = AssetCatalog::new();
        c.begin_download("a", 1, 10).unwrap();
        assert_eq!(c.begin_download("a", 2, 10), Err(AssetError::Busy));
        c.progress("a", 10).unwrap();
        c.finish_download("a", 3).unwrap();
        assert_eq!(c.begin_download("a", 2, 10), Err(AssetError::Busy));
        c.begin_compile("a").unwrap();
        assert_eq!(c.begin_download("a", 2, 10), Err(AssetError::Busy));
    }

    #[test]
    fn a_ready_asset_cannot_fail_and_removal_is_reported() {
        let mut c = AssetCatalog::new();
        ready_at(&mut c, "b", 1);
        ready_at(&mut c, "a", 1);
        assert_eq!(c.fail("b", "x"), Err(AssetError::Transition { phase: "ready", action: "fail" }));
        assert_eq!(c.names(), vec!["a", "b"]);
        assert!(c.remove("a"));
        assert!(!c.remove("a"));
        assert_eq!((c.phase("a"), c.ready_version("a")), (None, None));
    }
}
