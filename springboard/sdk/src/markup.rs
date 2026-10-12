//! Markup canvases for apps (`PaperMarkup`, the insertion menu of `MarkupEditViewController`, `FeatureSet` and
//! `MarkupAutoresizing`). [`MarkupCanvas`] holds the element model from `bifrost_ui::markup` and checks every element
//! it takes in against its feature set and its size rules. Drawing and pointer input belong to the shell; the canvas
//! stores the elements an app places and saves them as text that reads back exactly.
//!
//! Gap (see docs/parity/w4-paperkit.md): no shell view controller is exposed to apps yet, so there is no interactive
//! editor; apps place elements themselves (for example from [`Insertion`] menu items) and show them with their kit.

pub use bifrost_ui::markup::{
    Autoresizing, FeatureSet, ImageMarkup, Insertion, LinkMarkup, LoupeMarkup, MarkupDocument, MarkupElement,
    MarkupError, MarkupId, ShapeKind, ShapeMarkup,
};

/// A markup canvas of a fixed feature set and size rules (`PaperMarkup` with `FeatureSet` and `MarkupAutoresizing`).
/// Undo and redo are the document's; the canvas size is not part of the history, so an undo never shrinks it.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkupCanvas {
    doc: MarkupDocument,
    features: FeatureSet,
    resizing: Autoresizing,
    width: f32,
    height: f32,
}

impl MarkupCanvas {
    /// An empty canvas. The size must be finite and not negative.
    pub fn new(features: FeatureSet, resizing: Autoresizing, width: f32, height: f32) -> Result<Self, MarkupError> {
        check_size(width, height)?;
        Ok(MarkupCanvas { doc: MarkupDocument::new(), features, resizing, width, height })
    }

    pub fn features(&self) -> FeatureSet {
        self.features
    }

    pub fn document(&self) -> &MarkupDocument {
        &self.doc
    }

    /// The current width and height, after auto-resizing.
    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    /// Inserts an insertion-menu item with its top left corner at `origin`. `payload` is the text, asset name or URL the
    /// item needs (see [`Insertion::element`]). Fails with `FeatureDisabled` when the feature set does not allow the item.
    pub fn insert(&mut self, item: Insertion, origin: (f32, f32), payload: &str) -> Result<MarkupId, MarkupError> {
        self.insert_element(item.element(origin, payload))
    }

    /// Inserts an element the app built itself, such as an image from its own assets. The feature set must allow its
    /// kind, and the document checks its values. The canvas then grows if its auto-resizing allows it.
    pub fn insert_element(&mut self, element: MarkupElement) -> Result<MarkupId, MarkupError> {
        if !self.features.permits(&element) {
            return Err(MarkupError::FeatureDisabled(element.feature_name()));
        }
        let id = self.doc.insert(element)?;
        self.fit();
        Ok(id)
    }

    /// Takes an element out of the canvas. Its id stays retired, as in the document.
    pub fn remove(&mut self, id: MarkupId) -> Result<MarkupElement, MarkupError> {
        self.doc.remove(id)
    }

    pub fn undo(&mut self) -> bool {
        self.doc.undo()
    }

    pub fn redo(&mut self) -> bool {
        self.doc.redo()
    }

    /// Sets the size the app asks for and returns the size in effect. Auto-resizing may make it larger to hold the
    /// content; a flexible axis never goes below the content.
    pub fn set_size(&mut self, width: f32, height: f32) -> Result<(f32, f32), MarkupError> {
        check_size(width, height)?;
        self.width = width;
        self.height = height;
        self.fit();
        Ok(self.size())
    }

    /// The saved text of the elements (see `MarkupDocument::encode`). The size and rules are the app's to keep.
    pub fn save(&self) -> String {
        self.doc.encode()
    }

    /// Reads a saved document into a canvas with the given rules. An element the feature set does not allow is refused,
    /// as it would be on insert.
    pub fn load(
        text: &str,
        features: FeatureSet,
        resizing: Autoresizing,
        width: f32,
        height: f32,
    ) -> Result<Self, MarkupError> {
        check_size(width, height)?;
        let doc = MarkupDocument::decode(text)?;
        if let Some((_, element)) = doc.iter().find(|(_, el)| !features.permits(el)) {
            return Err(MarkupError::FeatureDisabled(element.feature_name()));
        }
        let mut canvas = MarkupCanvas { doc, features, resizing, width, height };
        canvas.fit();
        Ok(canvas)
    }

    fn fit(&mut self) {
        let (w, h) = self.resizing.fitted_size(&self.doc, self.width, self.height);
        self.width = w;
        self.height = h;
    }
}

fn check_size(width: f32, height: f32) -> Result<(), MarkupError> {
    if width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0 {
        Ok(())
    } else {
        Err(MarkupError::InvalidValue("canvas size"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROW_WIDTH: Autoresizing = Autoresizing { flexible_width: true, flexible_height: false };
    const GROW_HEIGHT: Autoresizing = Autoresizing { flexible_width: false, flexible_height: true };

    #[test]
    fn a_feature_set_refuses_elements_of_other_kinds_and_leaves_the_document_alone() {
        let mut canvas = MarkupCanvas::new(FeatureSet::SHAPES.union(FeatureSet::IMAGES), Autoresizing::FIXED, 300.0, 200.0)
            .unwrap();
        assert_eq!(canvas.insert(Insertion::Rectangle, (0.0, 0.0), "").map(|id| id.raw()), Ok(1));
        assert_eq!(canvas.insert(Insertion::Link, (0.0, 0.0), "https://example.org"), Err(MarkupError::FeatureDisabled("links")));
        assert_eq!(canvas.insert(Insertion::TextBox, (0.0, 0.0), "note"), Err(MarkupError::FeatureDisabled("text boxes")));
        assert_eq!(canvas.document().len(), 1, "refused items add nothing");
        assert!(canvas.insert(Insertion::Image, (0.0, 0.0), "shot.png").is_ok());
        assert_eq!(canvas.document().len(), 2);
    }

    #[test]
    fn an_insertion_menu_item_places_its_default_element_at_the_origin() {
        let mut canvas = MarkupCanvas::new(FeatureSet::ALL, Autoresizing::FIXED, 1000.0, 1000.0).unwrap();
        let id = canvas.insert(Insertion::Image, (10.0, 20.0), "shot.png").unwrap();
        assert_eq!(
            canvas.document().get(id),
            Some(&MarkupElement::Image(ImageMarkup {
                frame: bifrost_ui::Rect { x: 10.0, y: 20.0, w: 160.0, h: 120.0 },
                source: "shot.png".into(),
            }))
        );
        assert_eq!(canvas.insert(Insertion::Image, (0.0, 0.0), ""), Err(MarkupError::InvalidValue("image source")));
    }

    #[test]
    fn flexible_width_grows_the_canvas_to_the_content_and_never_shrinks_it() {
        let mut canvas = MarkupCanvas::new(FeatureSet::ALL, GROW_WIDTH, 300.0, 200.0).unwrap();
        canvas.insert(Insertion::Rectangle, (250.0, 10.0), "").unwrap();
        assert_eq!(canvas.size(), (370.0, 200.0), "right edge 250 + 120");
        canvas.insert(Insertion::Rectangle, (0.0, 0.0), "").unwrap();
        assert_eq!(canvas.size(), (370.0, 200.0), "content inside the canvas changes nothing");
        assert_eq!(canvas.set_size(100.0, 100.0), Ok((370.0, 100.0)), "a request below the content keeps the content width");
    }

    #[test]
    fn flexible_height_grows_the_canvas_for_an_element_below_the_edge() {
        let mut canvas = MarkupCanvas::new(FeatureSet::ALL, GROW_HEIGHT, 300.0, 200.0).unwrap();
        canvas.insert(Insertion::Ellipse, (0.0, 190.0), "").unwrap();
        assert_eq!(canvas.size(), (300.0, 270.0), "bottom edge 190 + 80");
    }

    #[test]
    fn undo_removes_content_but_the_canvas_keeps_its_grown_size() {
        let mut canvas = MarkupCanvas::new(FeatureSet::ALL, GROW_WIDTH, 300.0, 200.0).unwrap();
        canvas.insert(Insertion::Rectangle, (250.0, 10.0), "").unwrap();
        assert!(canvas.undo());
        assert!(canvas.document().is_empty());
        assert_eq!(canvas.size(), (370.0, 200.0), "size is not part of the undo history");
        assert!(canvas.redo());
        assert_eq!(canvas.document().len(), 1);
    }

    #[test]
    fn a_saved_canvas_loads_only_under_a_feature_set_that_allows_its_elements() {
        let mut canvas = MarkupCanvas::new(FeatureSet::ALL, Autoresizing::FIXED, 300.0, 200.0).unwrap();
        canvas.insert(Insertion::Loupe, (5.0, 5.0), "").unwrap();
        let saved = canvas.save();
        let refused = MarkupCanvas::load(&saved, FeatureSet::SHAPES, Autoresizing::FIXED, 300.0, 200.0);
        assert_eq!(refused, Err(MarkupError::FeatureDisabled("loupes")));
        let loaded = MarkupCanvas::load(&saved, FeatureSet::ALL, Autoresizing::FIXED, 300.0, 200.0).unwrap();
        assert_eq!(loaded.document().len(), 1);
        assert_eq!(loaded.save(), saved, "the text reads back to the same text");
    }

    #[test]
    fn a_canvas_rejects_sizes_that_are_not_finite_or_are_negative() {
        assert_eq!(
            MarkupCanvas::new(FeatureSet::ALL, Autoresizing::FIXED, -1.0, 10.0),
            Err(MarkupError::InvalidValue("canvas size"))
        );
        let mut canvas = MarkupCanvas::new(FeatureSet::ALL, Autoresizing::FIXED, 10.0, 10.0).unwrap();
        assert_eq!(canvas.set_size(f32::NAN, 10.0), Err(MarkupError::InvalidValue("canvas size")));
        assert_eq!(canvas.size(), (10.0, 10.0), "a refused size changes nothing");
    }
}
