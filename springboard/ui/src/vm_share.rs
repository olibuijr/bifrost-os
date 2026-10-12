//! Host directories shared with a guest virtual machine. A file system device carries a tag that names it inside the
//! guest. Its share lists the host directories it exposes: one directory, or several named directories, each either
//! read-only or read-write. The single and multiple shapes follow the official Virtualization documentation for the
//! file system device. The tag and name rules are BifrOSt's own, because that page does not state them.
//! This is a pure model: nothing is exported or mounted here.

/// Longest tag, in bytes.
pub const MAX_TAG_BYTES: usize = 36;

/// Why a tag, path, name or share was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareError {
    /// A tag is 1 to 36 bytes of ASCII letters, digits, '-', '_' or '.', and does not start with '-' or '.'.
    InvalidTag,
    /// A host path must be absolute and must not contain NUL.
    InvalidPath,
    /// A name in a multiple-directory share is non-empty, has no '/' or NUL, and is neither '.' nor '..'.
    InvalidName,
    /// Two directories in one multiple-directory share use the same name.
    DuplicateName,
    /// A multiple-directory share with no directories.
    EmptyShare,
}

/// Checks a device tag.
pub fn validate_tag(tag: &str) -> Result<(), ShareError> {
    if tag.is_empty() || tag.len() > MAX_TAG_BYTES {
        return Err(ShareError::InvalidTag);
    }
    if tag.starts_with('-') || tag.starts_with('.') {
        return Err(ShareError::InvalidTag);
    }
    if !tag
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ShareError::InvalidTag);
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), ShareError> {
    if path.starts_with('/') && !path.contains('\0') {
        Ok(())
    } else {
        Err(ShareError::InvalidPath)
    }
}

fn validate_name(name: &str) -> Result<(), ShareError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        Err(ShareError::InvalidName)
    } else {
        Ok(())
    }
}

/// A host directory and whether the guest may only read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedDirectory {
    pub path: String,
    pub read_only: bool,
}

/// What a device exposes: one directory, or named directories that the guest sees as subfolders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryShare {
    Single(SharedDirectory),
    Multiple(Vec<(String, SharedDirectory)>),
}

/// Checks a share before it is attached: paths absolute, names valid and unique, a multiple share not empty.
pub fn validate_share(share: &DirectoryShare) -> Result<(), ShareError> {
    match share {
        DirectoryShare::Single(dir) => validate_path(&dir.path),
        DirectoryShare::Multiple(list) => {
            if list.is_empty() {
                return Err(ShareError::EmptyShare);
            }
            for (name, dir) in list {
                validate_name(name)?;
                validate_path(&dir.path)?;
            }
            for i in 0..list.len() {
                if list[i + 1..].iter().any(|(other, _)| *other == list[i].0) {
                    return Err(ShareError::DuplicateName);
                }
            }
            Ok(())
        }
    }
}

/// A file system device: a tag plus at most one share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSystemDevice {
    tag: String,
    share: Option<DirectoryShare>,
}

impl FileSystemDevice {
    /// Creates a device with no share. Refuses an invalid tag.
    pub fn new(tag: &str) -> Result<Self, ShareError> {
        validate_tag(tag)?;
        Ok(Self {
            tag: tag.to_string(),
            share: None,
        })
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn share(&self) -> Option<&DirectoryShare> {
        self.share.as_ref()
    }

    /// Attaches a share. An invalid share is refused and the device keeps its previous share.
    pub fn set_share(&mut self, share: DirectoryShare) -> Result<(), ShareError> {
        validate_share(&share)?;
        self.share = Some(share);
        Ok(())
    }

    pub fn clear_share(&mut self) {
        self.share = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &str, read_only: bool) -> SharedDirectory {
        SharedDirectory {
            path: path.to_string(),
            read_only,
        }
    }

    #[test]
    fn tag_accepts_letters_digits_and_punctuation() {
        assert_eq!(validate_tag("home_1.share-x"), Ok(()));
        assert_eq!(validate_tag("A"), Ok(()));
    }

    #[test]
    fn tag_length_is_limited_to_36_bytes() {
        assert_eq!(validate_tag(&"a".repeat(36)), Ok(()));
        assert_eq!(validate_tag(&"a".repeat(37)), Err(ShareError::InvalidTag));
    }

    #[test]
    fn tag_rejects_empty_leading_punctuation_and_spaces() {
        assert_eq!(validate_tag(""), Err(ShareError::InvalidTag));
        assert_eq!(validate_tag("-x"), Err(ShareError::InvalidTag));
        assert_eq!(validate_tag(".hidden"), Err(ShareError::InvalidTag));
        assert_eq!(validate_tag("my share"), Err(ShareError::InvalidTag));
    }

    #[test]
    fn single_share_needs_an_absolute_path() {
        let mut dev = FileSystemDevice::new("projects").unwrap();
        assert_eq!(
            dev.set_share(DirectoryShare::Single(dir("Projects", false))),
            Err(ShareError::InvalidPath)
        );
        assert!(dev.share().is_none());
        assert_eq!(
            dev.set_share(DirectoryShare::Single(dir("/home/u/Projects", false))),
            Ok(())
        );
        assert!(dev.share().is_some());
    }

    #[test]
    fn multiple_share_rejects_bad_names_duplicates_and_empty_lists() {
        let ok = DirectoryShare::Multiple(vec![
            ("Invoices".to_string(), dir("/a", false)),
            ("Projects".to_string(), dir("/b", true)),
        ]);
        assert_eq!(validate_share(&ok), Ok(()));

        let slash = DirectoryShare::Multiple(vec![("a/b".to_string(), dir("/a", false))]);
        assert_eq!(validate_share(&slash), Err(ShareError::InvalidName));

        let dots = DirectoryShare::Multiple(vec![("..".to_string(), dir("/a", false))]);
        assert_eq!(validate_share(&dots), Err(ShareError::InvalidName));

        let dup = DirectoryShare::Multiple(vec![
            ("Notes".to_string(), dir("/a", false)),
            ("Notes".to_string(), dir("/b", false)),
        ]);
        assert_eq!(validate_share(&dup), Err(ShareError::DuplicateName));

        assert_eq!(
            validate_share(&DirectoryShare::Multiple(Vec::new())),
            Err(ShareError::EmptyShare)
        );
    }

    #[test]
    fn read_only_flag_is_kept_and_a_refused_share_keeps_the_old_one() {
        let mut dev = FileSystemDevice::new("docs").unwrap();
        dev.set_share(DirectoryShare::Single(dir("/srv/docs", true)))
            .unwrap();
        match dev.share() {
            Some(DirectoryShare::Single(d)) => assert!(d.read_only),
            other => panic!("unexpected share {:?}", other),
        }
        let refused = dev.set_share(DirectoryShare::Single(dir("relative", false)));
        assert_eq!(refused, Err(ShareError::InvalidPath));
        assert!(matches!(dev.share(), Some(DirectoryShare::Single(d)) if d.path == "/srv/docs"));
        dev.clear_share();
        assert!(dev.share().is_none());
        assert_eq!(dev.tag(), "docs");
    }
}
