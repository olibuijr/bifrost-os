//! Asset packs (`AssetPackManager`, `AssetPackManifest`, `AssetPack`): the manifest of packs an app can download, the
//! local state of each pack (version, removal, updates), language-matched pack selection and installed sizes
//! (`SizeCalculationMethod`). Pure state with no I/O: the downloader reports finished packs through `mark_downloaded`.

use std::collections::HashMap;

/// How the size of an installed file is counted (`SizeCalculationMethod`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeCalculationMethod {
    /// The number of bytes in each file.
    Logical,
    /// The bytes the file system allocates for each file: its length rounded up to whole blocks.
    Allocated,
}

impl SizeCalculationMethod {
    /// Size of one file of `file_bytes` on a file system with `block_size`-byte allocation units.
    /// A block size of 0 means the file system does not allocate blocks, so the logical size is used.
    pub fn file_size(self, file_bytes: u64, block_size: u64) -> u64 {
        match self {
            Self::Logical => file_bytes,
            Self::Allocated if block_size == 0 => file_bytes,
            Self::Allocated => file_bytes.div_ceil(block_size) * block_size,
        }
    }
}

/// One downloadable pack: its id, version, download size, the length of each file it installs, and the languages it
/// is localized for (`en`, `is_IS`). An empty `languages` list means the pack is not localized and always wanted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPack {
    pub id: String,
    pub version: u32,
    pub download_bytes: u64,
    pub file_sizes: Vec<u64>,
    pub languages: Vec<String>,
}

impl AssetPack {
    /// Installed size in logical bytes.
    pub fn logical_bytes(&self) -> u64 {
        self.file_sizes.iter().sum()
    }

    /// Installed size counted with `method`.
    pub fn size(&self, method: SizeCalculationMethod, block_size: u64) -> u64 {
        self.file_sizes.iter().map(|&bytes| method.file_size(bytes, block_size)).sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    EmptyId,
    Duplicate(String),
    /// No pack with this id in the manifest (`assetPackNotFound(withID:)`).
    UnknownPack(String),
    /// The pack is in the manifest but not usable on this device at its current version.
    NotAvailable(String),
    /// A saved manifest line could not be read; the field is the 1-based line number.
    Malformed(usize),
}

/// The result of `AssetPackManager::ensure_local_availability` (`AssetPackManager.LocalAvailabilityError`): the ids
/// that are available locally, and for each other requested id the reason it is not. Both lists keep request order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAvailabilityError {
    pub successes: Vec<String>,
    pub failures: Vec<(String, PackError)>,
}

/// The packs an app can download (`AssetPackManifest`). Ids are unique and not empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPackManifest {
    packs: Vec<AssetPack>,
}

impl AssetPackManifest {
    pub fn new(packs: Vec<AssetPack>) -> Result<Self, PackError> {
        let mut seen: Vec<&str> = Vec::with_capacity(packs.len());
        for pack in &packs {
            if pack.id.is_empty() {
                return Err(PackError::EmptyId);
            }
            if seen.contains(&pack.id.as_str()) {
                return Err(PackError::Duplicate(pack.id.clone()));
            }
            seen.push(&pack.id);
        }
        Ok(Self { packs })
    }

    /// `allAssetPacks`: every pack in manifest order.
    pub fn all_asset_packs(&self) -> &[AssetPack] {
        &self.packs
    }

    /// `assetPack(withID:)`.
    pub fn asset_pack(&self, id: &str) -> Option<&AssetPack> {
        self.packs.iter().find(|pack| pack.id == id)
    }

    /// Writes the manifest as one tab-separated line per pack: id, version, download bytes, file sizes separated by
    /// commas, languages separated by commas. Ids and languages must not contain tabs, commas or newlines.
    pub fn to_lines(&self) -> String {
        let mut out = String::new();
        for pack in &self.packs {
            let files: Vec<String> = pack.file_sizes.iter().map(|b| b.to_string()).collect();
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                pack.id,
                pack.version,
                pack.download_bytes,
                files.join(","),
                pack.languages.join(",")
            ));
        }
        out
    }

    /// Reads a manifest written by `to_lines`. The same checks as `new` apply to the ids.
    pub fn from_lines(text: &str) -> Result<Self, PackError> {
        let mut packs = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line_no = n + 1;
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 5 {
                return Err(PackError::Malformed(line_no));
            }
            let version: u32 = fields[1].parse().map_err(|_| PackError::Malformed(line_no))?;
            let download_bytes: u64 = fields[2].parse().map_err(|_| PackError::Malformed(line_no))?;
            let file_sizes: Result<Vec<u64>, _> = split_list(fields[3]).map(|s| s.parse::<u64>()).collect();
            let file_sizes = file_sizes.map_err(|_| PackError::Malformed(line_no))?;
            packs.push(AssetPack {
                id: fields[0].to_string(),
                version,
                download_bytes,
                file_sizes,
                languages: split_list(fields[4]).map(str::to_string).collect(),
            });
        }
        Self::new(packs)
    }
}

/// The items of a comma-separated field. An empty field has no items.
fn split_list<'a>(field: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    field.split(',').filter(|s| !s.is_empty())
}

/// The state of one pack on this device (`AssetPack.Status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackStatus {
    /// Not on this device.
    NotDownloaded,
    /// On this device at the manifest's version.
    Available,
    /// On this device, but the manifest has a newer version.
    UpdateAvailable,
}

/// Language tag to its language part: `is_IS` and `is-IS` both give `is`.
fn language_of(tag: &str) -> &str {
    tag.split(['_', '-']).next().unwrap_or("")
}

/// Local asset-pack state over a manifest (`AssetPackManager`). Packs are tracked by id with the installed version.
pub struct AssetPackManager {
    manifest: AssetPackManifest,
    installed: HashMap<String, u32>,
}

impl AssetPackManager {
    pub fn new(manifest: AssetPackManifest) -> Self {
        Self { manifest, installed: HashMap::new() }
    }

    pub fn manifest(&self) -> &AssetPackManifest {
        &self.manifest
    }

    /// `status(ofAssetPackWithID:)`.
    pub fn status(&self, id: &str) -> Result<PackStatus, PackError> {
        let pack = self.manifest.asset_pack(id).ok_or_else(|| PackError::UnknownPack(id.to_string()))?;
        Ok(match self.installed.get(id) {
            None => PackStatus::NotDownloaded,
            Some(&version) if version < pack.version => PackStatus::UpdateAvailable,
            Some(_) => PackStatus::Available,
        })
    }

    /// Records that the pack was downloaded and installed at the manifest's current version.
    pub fn mark_downloaded(&mut self, id: &str) -> Result<(), PackError> {
        let version = self.manifest.asset_pack(id).ok_or_else(|| PackError::UnknownPack(id.to_string()))?.version;
        self.installed.insert(id.to_string(), version);
        Ok(())
    }

    /// Restores the installed version of a pack that an earlier run downloaded (the local state an app saved). The
    /// pack must be in the manifest; a version above the manifest's is kept as is and reads as available.
    pub fn set_installed(&mut self, id: &str, version: u32) -> Result<(), PackError> {
        if self.manifest.asset_pack(id).is_none() {
            return Err(PackError::UnknownPack(id.to_string()));
        }
        self.installed.insert(id.to_string(), version);
        Ok(())
    }

    /// The installed version of a pack, `None` when it is not on the device.
    pub fn installed_version(&self, id: &str) -> Option<u32> {
        self.installed.get(id).copied()
    }

    /// `remove(assetPackWithID:)`: returns the logical bytes freed. A pack that is not on the device frees nothing.
    pub fn remove(&mut self, id: &str) -> Result<u64, PackError> {
        let pack = self.manifest.asset_pack(id).ok_or_else(|| PackError::UnknownPack(id.to_string()))?;
        Ok(if self.installed.remove(id).is_some() { pack.logical_bytes() } else { 0 })
    }

    /// `localSize(ofAssetPackWithID:calculationMethod:)`: `None` when the pack is not on the device.
    pub fn local_size(&self, id: &str, method: SizeCalculationMethod, block_size: u64) -> Option<u64> {
        if !self.installed.contains_key(id) {
            return None;
        }
        self.manifest.asset_pack(id).map(|pack| pack.size(method, block_size))
    }

    /// `ensureLocalAvailability(of:)` planning: the ids in `ids` that are not yet available at the current version.
    pub fn needed(&self, ids: &[&str]) -> Result<Vec<String>, PackError> {
        let mut missing = Vec::new();
        for id in ids {
            if self.status(id)? != PackStatus::Available {
                missing.push(id.to_string());
            }
        }
        Ok(missing)
    }

    /// `ensureLocalAvailability(of:)` result without a download: every id that is available at its current version is
    /// a success. An unknown id fails with `UnknownPack`, and a pack that is missing or outdated fails with
    /// `NotAvailable`.
    pub fn ensure_local_availability(&self, ids: &[&str]) -> LocalAvailabilityError {
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        for &id in ids {
            match self.status(id) {
                Ok(PackStatus::Available) => successes.push(id.to_string()),
                Ok(_) => failures.push((id.to_string(), PackError::NotAvailable(id.to_string()))),
                Err(error) => failures.push((id.to_string(), error)),
            }
        }
        LocalAvailabilityError { successes, failures }
    }

    fn localized_for(&self, language: &str) -> bool {
        !language.is_empty()
            && self
                .manifest
                .all_asset_packs()
                .iter()
                .any(|pack| pack.languages.iter().any(|l| l.eq_ignore_ascii_case(language)))
    }

    /// `resolvedLanguage`: the first preferred language (most preferred first) that some localized pack is made for.
    /// Region tags match their language, so `is_IS` selects packs made for `is`.
    pub fn resolved_language(&self, preferred: &[&str]) -> Option<String> {
        for tag in preferred {
            let language = language_of(tag);
            if self.localized_for(language) {
                return Some(language.to_string());
            }
        }
        None
    }

    /// The packs to fetch for `preferred` languages: every unlocalized pack, plus the localized packs for the resolved
    /// language. Packs localized for other languages are left out.
    pub fn packs_for_languages(&self, preferred: &[&str]) -> Vec<&AssetPack> {
        let chosen = self.resolved_language(preferred);
        self.manifest
            .all_asset_packs()
            .iter()
            .filter(|pack| {
                pack.languages.is_empty()
                    || chosen.as_deref().is_some_and(|c| pack.languages.iter().any(|l| l.eq_ignore_ascii_case(c)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(id: &str, version: u32, languages: &[&str], files: &[u64]) -> AssetPack {
        AssetPack {
            id: id.to_string(),
            version,
            download_bytes: files.iter().sum::<u64>() / 2,
            file_sizes: files.to_vec(),
            languages: languages.iter().map(|l| l.to_string()).collect(),
        }
    }

    fn manager(packs: Vec<AssetPack>) -> AssetPackManager {
        AssetPackManager::new(AssetPackManifest::new(packs).expect("valid manifest"))
    }

    #[test]
    fn manifest_rejects_empty_and_duplicate_ids() {
        assert_eq!(AssetPackManifest::new(vec![pack("", 1, &[], &[1])]), Err(PackError::EmptyId));
        let dup = vec![pack("level1", 1, &[], &[1]), pack("level1", 2, &[], &[2])];
        assert_eq!(AssetPackManifest::new(dup), Err(PackError::Duplicate("level1".into())));
        let ok = AssetPackManifest::new(vec![pack("a", 1, &[], &[1]), pack("b", 1, &[], &[1])]).unwrap();
        assert_eq!(ok.all_asset_packs().len(), 2);
        assert_eq!(ok.asset_pack("b").map(|p| p.id.as_str()), Some("b"));
        assert!(ok.asset_pack("c").is_none());
    }

    #[test]
    fn status_follows_download_and_manifest_version() {
        let mut m = manager(vec![pack("tutorial", 1, &[], &[100])]);
        assert_eq!(m.status("tutorial"), Ok(PackStatus::NotDownloaded));
        m.mark_downloaded("tutorial").unwrap();
        assert_eq!(m.status("tutorial"), Ok(PackStatus::Available));
        // A newer manifest version makes the installed copy outdated.
        let newer = AssetPackManifest::new(vec![pack("tutorial", 2, &[], &[100])]).unwrap();
        let mut bumped = AssetPackManager::new(newer);
        bumped.installed.insert("tutorial".into(), 1);
        assert_eq!(bumped.status("tutorial"), Ok(PackStatus::UpdateAvailable));
        assert_eq!(m.status("missing"), Err(PackError::UnknownPack("missing".into())));
    }

    #[test]
    fn remove_frees_logical_bytes_once() {
        let mut m = manager(vec![pack("levels", 1, &[], &[300, 700])]);
        assert_eq!(m.remove("levels"), Ok(0), "a pack that is not installed frees nothing");
        m.mark_downloaded("levels").unwrap();
        assert_eq!(m.remove("levels"), Ok(1000));
        assert_eq!(m.status("levels"), Ok(PackStatus::NotDownloaded));
        assert_eq!(m.remove("levels"), Ok(0));
        assert_eq!(m.remove("nope"), Err(PackError::UnknownPack("nope".into())));
    }

    #[test]
    fn allocated_size_rounds_each_file_up_to_whole_blocks() {
        assert_eq!(SizeCalculationMethod::Allocated.file_size(5000, 4096), 8192);
        assert_eq!(SizeCalculationMethod::Allocated.file_size(4096, 4096), 4096);
        assert_eq!(SizeCalculationMethod::Logical.file_size(5000, 4096), 5000);
        assert_eq!(SizeCalculationMethod::Allocated.file_size(5000, 0), 5000);
        let mut m = manager(vec![pack("art", 1, &[], &[5000, 10])]);
        assert_eq!(m.local_size("art", SizeCalculationMethod::Allocated, 4096), None);
        m.mark_downloaded("art").unwrap();
        assert_eq!(m.local_size("art", SizeCalculationMethod::Allocated, 4096), Some(8192 + 4096));
        assert_eq!(m.local_size("art", SizeCalculationMethod::Logical, 4096), Some(5010));
    }

    #[test]
    fn resolved_language_honours_preference_order_and_region_tags() {
        let m = manager(vec![pack("en-pack", 1, &["en"], &[1]), pack("is-pack", 1, &["is"], &[1])]);
        assert_eq!(m.resolved_language(&["is_IS", "en"]), Some("is".to_string()));
        assert_eq!(m.resolved_language(&["de", "en-US"]), Some("en".to_string()));
        assert_eq!(m.resolved_language(&["de", "fr"]), None);
        assert_eq!(m.resolved_language(&[]), None);
    }

    #[test]
    fn packs_for_languages_keep_unlocalized_and_chosen_language_only() {
        let m = manager(vec![
            pack("core", 1, &[], &[1]),
            pack("text-en", 1, &["en"], &[1]),
            pack("text-is", 1, &["is"], &[1]),
        ]);
        let ids: Vec<&str> = m.packs_for_languages(&["is_IS"]).iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["core", "text-is"]);
        let none: Vec<&str> = m.packs_for_languages(&["de"]).iter().map(|p| p.id.as_str()).collect();
        assert_eq!(none, vec!["core"]);
    }

    #[test]
    fn needed_lists_only_packs_not_yet_available() {
        let mut m = manager(vec![pack("a", 1, &[], &[1]), pack("b", 1, &[], &[1]), pack("c", 1, &[], &[1])]);
        m.mark_downloaded("b").unwrap();
        assert_eq!(m.needed(&["a", "b", "c"]), Ok(vec!["a".to_string(), "c".to_string()]));
        assert_eq!(m.needed(&["a", "zzz"]), Err(PackError::UnknownPack("zzz".into())));
    }

    #[test]
    fn local_availability_splits_successes_from_reasons() {
        let mut m = manager(vec![pack("a", 1, &[], &[1]), pack("b", 1, &[], &[1]), pack("c", 1, &[], &[1])]);
        m.mark_downloaded("a").unwrap();
        // Installed at version 0 while the manifest is at 1: outdated, so not available.
        m.installed.insert("c".into(), 0);
        let result = m.ensure_local_availability(&["a", "b", "c", "zz"]);
        assert_eq!(result.successes, vec!["a".to_string()]);
        assert_eq!(
            result.failures,
            vec![
                ("b".to_string(), PackError::NotAvailable("b".into())),
                ("c".to_string(), PackError::NotAvailable("c".into())),
                ("zz".to_string(), PackError::UnknownPack("zz".into())),
            ]
        );
    }

    #[test]
    fn manifest_lines_round_trip_files_and_languages() {
        // "core": download is (5000 + 10) / 2 = 2505 by the helper; "text" has no files and no languages filter.
        let packs = vec![pack("core", 3, &[], &[5000, 10]), pack("text", 1, &["en", "is"], &[])];
        let manifest = AssetPackManifest::new(packs).unwrap();
        let text = manifest.to_lines();
        assert_eq!(text, "core\t3\t2505\t5000,10\t\ntext\t1\t0\t\ten,is\n");
        assert_eq!(AssetPackManifest::from_lines(&text), Ok(manifest));
    }

    #[test]
    fn manifest_lines_reject_bad_fields_with_their_line_number() {
        assert_eq!(AssetPackManifest::from_lines("a\t1\t10\t5\n"), Err(PackError::Malformed(1)));
        assert_eq!(AssetPackManifest::from_lines("a\tx\t10\t5\t\n"), Err(PackError::Malformed(1)));
        assert_eq!(AssetPackManifest::from_lines("a\t1\t10\t5,y\t\n"), Err(PackError::Malformed(1)));
        // The empty second line is skipped but still counted, so the bad third line is reported as line 3.
        assert_eq!(AssetPackManifest::from_lines("a\t1\t1\t\t\n\nb\t1\tbad\t\t\n"), Err(PackError::Malformed(3)));
        assert_eq!(AssetPackManifest::from_lines("\t1\t10\t\t\n"), Err(PackError::EmptyId));
        assert_eq!(
            AssetPackManifest::from_lines("a\t1\t1\t\t\na\t2\t2\t\t\n"),
            Err(PackError::Duplicate("a".into()))
        );
    }
}
