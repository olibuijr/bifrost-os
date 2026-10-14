//! The kind of media a file holds, decided from its name. The image, audio and movie extensions are one table for every
//! app that lists media, so the apps agree on which files are media.
//!
//! The kind is the primary kind of [`SourceType`], the same kind a media source declares. A file name is media when its
//! extension is in one of the three tables, compared without case. A name that starts with a dot is a hidden file and is
//! never media, and so is a name with no stem before its extension. Only the last path component is read, so a directory
//! called `take.mp4` does not make the files inside it movies. The tables name formats by extension only: the file's
//! bytes are not sniffed here, and a decoder that finds a mislabelled file reports it itself.

use crate::media_sources::SourceType;

/// Still picture extensions.
const IMAGE_EXTENSIONS: &[&str] = &["bmp", "gif", "heic", "heif", "jpeg", "jpg", "png", "tif", "tiff", "webp"];

/// Audio extensions, including the Standard MIDI File extensions.
const AUDIO_EXTENSIONS: &[&str] = &["aac", "aiff", "flac", "m4a", "mid", "midi", "mp3", "ogg", "opus", "wav"];

/// Moving picture extensions.
const MOVIE_EXTENSIONS: &[&str] = &["avi", "m4v", "mkv", "mov", "mp4", "webm"];

/// The kind of media that `name` holds, or `None` when the name is not media (no extension in the tables, a hidden file,
/// or a name with an empty stem).
pub fn kind_of_name(name: &str) -> Option<SourceType> {
    let file = name.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(name);
    if file.starts_with('.') {
        return None;
    }
    let (stem, extension) = file.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let extension = extension.to_ascii_lowercase();
    let extension = extension.as_str();
    if IMAGE_EXTENSIONS.contains(&extension) {
        Some(SourceType::Image)
    } else if AUDIO_EXTENSIONS.contains(&extension) {
        Some(SourceType::Audio)
    } else if MOVIE_EXTENSIONS.contains(&extension) {
        Some(SourceType::Movie)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_extension_table_maps_to_its_source_type() {
        assert_eq!(kind_of_name("holiday.JPG"), Some(SourceType::Image));
        assert_eq!(kind_of_name("scan.tiff"), Some(SourceType::Image));
        assert_eq!(kind_of_name("song.flac"), Some(SourceType::Audio));
        assert_eq!(kind_of_name("riff.MID"), Some(SourceType::Audio));
        assert_eq!(kind_of_name("clip.MOV"), Some(SourceType::Movie));
        assert_eq!(kind_of_name("clip.webm"), Some(SourceType::Movie));
    }

    #[test]
    fn names_outside_the_tables_are_not_media() {
        assert_eq!(kind_of_name("notes.txt"), None);
        assert_eq!(kind_of_name("archive.tar.gz"), None);
        assert_eq!(kind_of_name("README"), None);
        assert_eq!(kind_of_name("trailing."), None);
    }

    #[test]
    fn hidden_files_and_nameless_extensions_are_not_media() {
        assert_eq!(kind_of_name(".hidden.mp3"), None);
        assert_eq!(kind_of_name(".mp3"), None);
        assert_eq!(kind_of_name("sub/.cache/a.png"), Some(SourceType::Image));
    }

    #[test]
    fn only_the_last_path_component_decides_the_kind() {
        assert_eq!(kind_of_name("take.mp4/readme.txt"), None);
        assert_eq!(kind_of_name("albums/track one.m4a"), Some(SourceType::Audio));
        assert_eq!(kind_of_name("albums\\track.wav"), Some(SourceType::Audio));
    }

    #[test]
    fn the_last_extension_counts_when_a_name_has_several() {
        assert_eq!(kind_of_name("backup.png.mp3"), Some(SourceType::Audio));
        assert_eq!(kind_of_name("cover.mp3.png"), Some(SourceType::Image));
    }
}
