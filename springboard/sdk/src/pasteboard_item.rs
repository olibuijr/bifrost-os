//! `PasteboardItem`: one copied thing offered in several typed representations. Types are UTType
//! ids; a pasteboard holds a list of items (`pasteboard::NativePasteboard::write_items`).

use bifrost_ui::uttype;

/// `PasteboardItem`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PasteboardItem {
    reps: Vec<(String, Vec<u8>)>,
}

impl PasteboardItem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an item from flat (type, data) pairs.
    pub fn from_representations(reps: Vec<(String, Vec<u8>)>) -> Self {
        let mut item = Self::new();
        for (t, d) in reps {
            item.set_data(&d, &t);
        }
        item
    }

    /// `setData(_:forType:)`: replaces the data of `uti`; returns true.
    pub fn set_data(&mut self, bytes: &[u8], uti: &str) -> bool {
        self.reps.retain(|(t, _)| t != uti);
        self.reps.push((uti.to_string(), bytes.to_vec()));
        true
    }

    /// `setString(_:forType:)`.
    pub fn set_string(&mut self, text: &str, uti: &str) -> bool {
        self.set_data(text.as_bytes(), uti)
    }

    /// `data(forType:)`.
    pub fn data(&self, uti: &str) -> Option<&[u8]> {
        self.reps.iter().find(|(t, _)| t == uti).map(|(_, d)| d.as_slice())
    }

    /// `string(forType:)`.
    pub fn string(&self, uti: &str) -> Option<String> {
        self.data(uti).and_then(|d| String::from_utf8(d.to_vec()).ok())
    }

    /// `types`, in the order they were set.
    pub fn types(&self) -> Vec<String> {
        self.reps.iter().map(|(t, _)| t.clone()).collect()
    }

    /// The flat (type, data) pairs.
    pub fn representations(&self) -> &[(String, Vec<u8>)] {
        &self.reps
    }

    /// True when any type of the item is, or descends from, one of `ancestors`.
    pub fn conforms_to_any(&self, ancestors: &[&str]) -> bool {
        self.reps.iter().any(|(t, _)| ancestors.iter().any(|a| uttype::conforms(t, a)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_keeps_one_representation_per_type_in_order() {
        let mut i = PasteboardItem::new();
        i.set_string("a", "public.utf8-plain-text");
        i.set_data(b"<b>a</b>", "public.html");
        i.set_string("b", "public.utf8-plain-text");
        assert_eq!(i.types(), ["public.html", "public.utf8-plain-text"]);
        assert_eq!(i.string("public.utf8-plain-text").as_deref(), Some("b"));
        assert_eq!(i.data("public.png"), None);
    }

    #[test]
    fn conformance_uses_the_type_hierarchy() {
        let mut i = PasteboardItem::new();
        i.set_string("a", "public.utf8-plain-text");
        assert!(i.conforms_to_any(&["public.text"]));
        assert!(!i.conforms_to_any(&["public.image"]));
    }
}
