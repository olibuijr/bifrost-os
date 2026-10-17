//! Bitmap icon sets for files and content types, and custom folder icons (`NSWorkspace.icon(forFile:)` and
//! `NSWorkspace.setIcon(_:forFile:)` as images). [`crate::workspace::icon_for_content_type`] names a glyph; this module
//! lets an app supply the picture itself: an [`ImageObject`] with one or more bitmap representations per content type.
//!
//! A lookup walks the type tree: a file of a type with no icon of its own uses the nearest registered ancestor, the
//! same nearest-first rule as the type conformance in `bifrost_ui::uttype`. The folder icon is the set for
//! `public.folder`. The representation to draw is chosen with [`ImageObject::best_rep`].
//!
//! ```ignore
//! let mut icons = IconImages::new();
//! icons.set_for_type("public.folder", ImageObject::named("project-folder"));
//! let rep = icons.rep_for_file("notes.txt", (32.0, 32.0), 2.0);
//! ```
//!
//! The sets are in memory and belong to the app; nothing is written to the shell's icon store here.

use crate::image_object::{ImageObject, ImageRep};
use bifrost_ui::uttype;

/// Content type id of folders (`public.folder`); its set is the custom folder icon.
pub const FOLDER: &str = "public.folder";

/// The bitmap icon sets an app registered, one per content type.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconImages {
    sets: Vec<(String, ImageObject)>,
}

impl IconImages {
    /// No icons registered: every lookup falls back to the glyph of [`crate::workspace::icon_for_content_type`].
    pub fn new() -> Self {
        IconImages::default()
    }

    /// Sets the icon for a content type, replacing one set earlier for the same type. An image with no representations
    /// is refused (`false`) and nothing changes.
    pub fn set_for_type(&mut self, uttype_id: &str, image: ImageObject) -> bool {
        if image.reps().is_empty() {
            return false;
        }
        match self.sets.iter_mut().find(|(t, _)| t == uttype_id) {
            Some(slot) => slot.1 = image,
            None => self.sets.push((uttype_id.to_string(), image)),
        }
        true
    }

    /// Removes the icon set for a content type; `false` when it had none.
    pub fn remove_for_type(&mut self, uttype_id: &str) -> bool {
        let before = self.sets.len();
        self.sets.retain(|(t, _)| t != uttype_id);
        self.sets.len() != before
    }

    /// Sets the custom folder icon.
    pub fn set_folder(&mut self, image: ImageObject) -> bool {
        self.set_for_type(FOLDER, image)
    }

    /// The icon set for a content type, or the nearest registered ancestor's (`None` when no ancestor has one).
    pub fn image_for_type(&self, uttype_id: &str) -> Option<&ImageObject> {
        self.sets
            .iter()
            .filter_map(|(t, image)| uttype::distance(uttype_id, t).map(|d| (d, image)))
            .min_by_key(|(d, _)| *d)
            .map(|(_, image)| image)
    }

    /// The icon set for a file, by the content type its name's extension gives. `None` for a name without a known
    /// extension, or when no set covers its type.
    pub fn image_for_file(&self, name: &str) -> Option<&ImageObject> {
        self.image_for_type(uttype::type_for_extension(name)?)
    }

    /// The representation to draw for a file at `target` points and `scale` pixels per point. `None` when no set
    /// covers the file.
    pub fn rep_for_file(&self, name: &str, target: (f32, f32), scale: f32) -> Option<&ImageRep> {
        self.image_for_file(name)?.best_rep(target, scale)
    }

    /// The representation to draw for the folder icon, or `None` when no custom folder icon is set.
    pub fn rep_for_folder(&self, target: (f32, f32), scale: f32) -> Option<&ImageRep> {
        self.image_for_type(FOLDER)?.best_rep(target, scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(name: &str, px: u32) -> ImageObject {
        let mut image = ImageObject::named(name);
        image.add_rep(ImageRep::bitmap(px, px, (16.0, 16.0)).unwrap());
        image.add_rep(ImageRep::bitmap(px * 2, px * 2, (16.0, 16.0)).unwrap());
        image
    }

    #[test]
    fn a_file_uses_its_own_type_then_the_nearest_ancestor() {
        let mut icons = IconImages::new();
        assert!(icons.set_for_type("public.image", set("image", 16)));
        assert!(icons.set_for_type("public.text", set("text", 16)));
        assert_eq!(icons.image_for_file("photo.png").and_then(|i| i.name()), Some("image"));
        assert_eq!(icons.image_for_file("notes.txt").and_then(|i| i.name()), Some("text"));
        assert!(icons.image_for_file("archive.zip").is_none(), "no set covers an archive");
        assert!(icons.image_for_file("noextension").is_none());
    }

    #[test]
    fn the_rep_for_a_target_and_scale_is_the_smallest_covering_bitmap() {
        let mut icons = IconImages::new();
        icons.set_for_type("public.image", set("image", 16));
        // 16 points at 2x needs 32 pixels: the 32 px rep covers it and is the smallest that does.
        assert_eq!(icons.rep_for_file("a.jpg", (16.0, 16.0), 2.0).unwrap().size, (16.0, 16.0));
        let rep = icons.rep_for_file("a.jpg", (16.0, 16.0), 2.0).unwrap();
        assert_eq!(rep.scale(), Some(2.0));
    }

    #[test]
    fn the_folder_icon_is_a_custom_set_and_refuses_empty_images() {
        let mut icons = IconImages::new();
        assert!(icons.rep_for_folder((16.0, 16.0), 1.0).is_none());
        assert!(!icons.set_folder(ImageObject::named("empty")), "an image without reps is refused");
        assert!(icons.set_folder(set("folder", 16)));
        assert_eq!(icons.image_for_type(FOLDER).and_then(|i| i.name()), Some("folder"));
        assert!(icons.rep_for_folder((16.0, 16.0), 1.0).is_some());
        assert!(icons.remove_for_type(FOLDER));
        assert!(!icons.remove_for_type(FOLDER));
    }

    #[test]
    fn replacing_a_type_keeps_one_set_for_it() {
        let mut icons = IconImages::new();
        icons.set_for_type("public.text", set("first", 16));
        icons.set_for_type("public.text", set("second", 32));
        assert_eq!(icons.image_for_type("public.text").and_then(|i| i.name()), Some("second"));
        assert_eq!(icons.sets.len(), 1);
    }
}
