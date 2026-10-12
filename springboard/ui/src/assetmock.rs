//! A local test source for asset packs: an in-memory stand-in for the download host, so an app can run a full pack
//! download without a network. Fixtures are kept in memory, each one is checked against the manifest's download size,
//! the source can be made unreachable, and `deliver` marks a served pack as installed in an `AssetPackManager`. No
//! sockets and no files: the bytes are whatever the test put in.

use std::collections::HashMap;

use crate::assetpack::{AssetPackManager, AssetPackManifest, PackError};

/// Why a local pack download failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MockError {
    /// The source is switched off, as if the host could not be reached.
    Unreachable,
    /// The manifest has the pack, but no bytes were stored for it.
    NoFixture(String),
    /// The stored bytes are not as long as the pack's download size.
    FixtureSizeMismatch { id: String, expected: u64, actual: u64 },
    Pack(PackError),
}

impl From<PackError> for MockError {
    fn from(error: PackError) -> Self {
        MockError::Pack(error)
    }
}

/// An in-memory pack host for tests (`Testing asset packs locally`). It serves the bytes of each pack in its manifest.
pub struct MockPackSource {
    manifest: AssetPackManifest,
    fixtures: HashMap<String, Vec<u8>>,
    reachable: bool,
}

impl MockPackSource {
    pub fn new(manifest: AssetPackManifest) -> Self {
        Self { manifest, fixtures: HashMap::new(), reachable: true }
    }

    /// Switches the source on or off. An unreachable source fails every fetch, including packs it has.
    pub fn set_reachable(&mut self, reachable: bool) {
        self.reachable = reachable;
    }

    /// Stores the bytes served for pack `id`. The id must be in the manifest and the bytes must be as long as the
    /// pack's download size. A stored fixture replaces the previous one.
    pub fn add_fixture(&mut self, id: &str, bytes: Vec<u8>) -> Result<(), MockError> {
        let expected = self.manifest.asset_pack(id).ok_or_else(|| PackError::UnknownPack(id.to_string()))?.download_bytes;
        let actual = bytes.len() as u64;
        if actual != expected {
            return Err(MockError::FixtureSizeMismatch { id: id.to_string(), expected, actual });
        }
        self.fixtures.insert(id.to_string(), bytes);
        Ok(())
    }

    /// Serves the bytes of pack `id` as a download would. Fails when the source is unreachable or has no fixture.
    pub fn fetch(&self, id: &str) -> Result<&[u8], MockError> {
        if !self.reachable {
            return Err(MockError::Unreachable);
        }
        self.fixtures.get(id).map(|bytes| bytes.as_slice()).ok_or_else(|| MockError::NoFixture(id.to_string()))
    }

    /// Downloads pack `id` into `manager`: fetches its bytes, then records it as installed at the manager's current
    /// version. Returns the number of bytes received.
    pub fn deliver(&self, manager: &mut AssetPackManager, id: &str) -> Result<u64, MockError> {
        let received = self.fetch(id)?.len() as u64;
        manager.mark_downloaded(id)?;
        Ok(received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assetpack::{AssetPack, PackStatus};

    fn pack(id: &str, version: u32, download_bytes: u64) -> AssetPack {
        AssetPack {
            id: id.to_string(),
            version,
            download_bytes,
            file_sizes: vec![download_bytes * 2],
            languages: Vec::new(),
        }
    }

    fn source() -> MockPackSource {
        let manifest = AssetPackManifest::new(vec![pack("level1", 1, 10), pack("skin", 1, 4)]).unwrap();
        MockPackSource::new(manifest)
    }

    #[test]
    fn fixture_must_match_the_download_size() {
        let mut s = source();
        assert_eq!(
            s.add_fixture("level1", vec![0; 9]),
            Err(MockError::FixtureSizeMismatch { id: "level1".into(), expected: 10, actual: 9 })
        );
        assert_eq!(s.add_fixture("level1", vec![0; 10]), Ok(()));
        assert_eq!(s.add_fixture("nope", vec![0; 1]), Err(MockError::Pack(PackError::UnknownPack("nope".into()))));
    }

    #[test]
    fn fetch_without_a_fixture_names_the_pack() {
        let s = source();
        assert_eq!(s.fetch("skin"), Err(MockError::NoFixture("skin".into())));
    }

    #[test]
    fn unreachable_source_fails_even_with_a_fixture() {
        let mut s = source();
        s.add_fixture("skin", vec![7; 4]).unwrap();
        s.set_reachable(false);
        assert_eq!(s.fetch("skin"), Err(MockError::Unreachable));
        s.set_reachable(true);
        assert_eq!(s.fetch("skin").map(|b| b.len()), Ok(4));
    }

    #[test]
    fn deliver_installs_the_pack_in_the_manager() {
        let mut s = source();
        s.add_fixture("level1", vec![1; 10]).unwrap();
        let mut m = AssetPackManager::new(AssetPackManifest::new(vec![pack("level1", 1, 10)]).unwrap());
        assert_eq!(m.status("level1"), Ok(PackStatus::NotDownloaded));
        assert_eq!(s.deliver(&mut m, "level1"), Ok(10));
        assert_eq!(m.status("level1"), Ok(PackStatus::Available));
    }

    #[test]
    fn deliver_leaves_the_manager_alone_when_the_fetch_fails() {
        let s = source();
        let mut m = AssetPackManager::new(AssetPackManifest::new(vec![pack("skin", 1, 4)]).unwrap());
        assert_eq!(s.deliver(&mut m, "skin"), Err(MockError::NoFixture("skin".into())));
        assert_eq!(m.status("skin"), Ok(PackStatus::NotDownloaded));
    }

    #[test]
    fn deliver_reports_a_pack_the_manager_does_not_know() {
        let mut s = source();
        s.add_fixture("skin", vec![0; 4]).unwrap();
        let mut m = AssetPackManager::new(AssetPackManifest::new(vec![pack("level1", 1, 10)]).unwrap());
        assert_eq!(s.deliver(&mut m, "skin"), Err(MockError::Pack(PackError::UnknownPack("skin".into()))));
    }
}
