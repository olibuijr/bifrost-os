//! Page annotations as data: the markup a PDF page carries (ink drawings, stamps, shapes, notes, text highlights, links
//! and form widgets). Each annotation has a subtype, bounds in page space, text contents, a colour and, for ink, a
//! drawing. A page keeps its annotations in stacking order, hit-tests points against them, and can hide them as a whole.
//! The model does not parse or write PDF files: the reader or writer maps its annotation objects onto these values.

use crate::markup_drawing::Drawing;
use crate::Rect;

/// The kinds of annotation, named as the reference SDK lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subtype {
    Circle,
    FreeText,
    Highlight,
    Ink,
    Line,
    Link,
    Popup,
    Square,
    Stamp,
    StrikeOut,
    Text,
    Underline,
    Widget,
}

impl Subtype {
    /// Whether the kind is markup a reader makes on the page. Links, form widgets and popup windows are not markup.
    pub fn is_markup(self) -> bool {
        !matches!(self, Subtype::Link | Subtype::Popup | Subtype::Widget)
    }
}

/// One annotation on a page. `drawing` is set for ink annotations only. `color` is passed through as the caller packed it.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub subtype: Subtype,
    pub bounds: Rect,
    pub contents: String,
    pub color: u32,
    pub drawing: Option<Drawing>,
}

impl Annotation {
    /// An annotation with empty contents, an opaque black colour (0xff000000) and no drawing.
    pub fn new(subtype: Subtype, bounds: Rect) -> Self {
        Annotation { subtype, bounds, contents: String::new(), color: 0xff00_0000, drawing: None }
    }

    /// An ink annotation over a drawing. Its bounds are the drawing's bounds. An empty drawing gives `None`.
    pub fn ink(drawing: Drawing, color: u32) -> Option<Self> {
        let bounds = drawing.bounds()?;
        Some(Annotation { subtype: Subtype::Ink, bounds, contents: String::new(), color, drawing: Some(drawing) })
    }
}

#[derive(Clone, Debug)]
struct Entry {
    id: u32,
    annotation: Annotation,
}

/// The annotations of one page, in stacking order: a later annotation draws on top of an earlier one.
#[derive(Clone, Debug)]
pub struct PageAnnotations {
    entries: Vec<Entry>,
    next_id: u32,
    displays: bool,
}

impl Default for PageAnnotations {
    fn default() -> Self {
        Self::new()
    }
}

impl PageAnnotations {
    /// An empty page that displays its annotations.
    pub fn new() -> Self {
        PageAnnotations { entries: Vec::new(), next_id: 1, displays: true }
    }

    /// Adds an annotation on top and returns its id. Ids start at 1 and are never reused, even after a removal.
    pub fn add(&mut self, annotation: Annotation) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(Entry { id, annotation });
        id
    }

    /// Removes an annotation and returns it, or `None` for an unknown id.
    pub fn remove(&mut self, id: u32) -> Option<Annotation> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(at).annotation)
    }

    pub fn get(&self, id: u32) -> Option<&Annotation> {
        self.entries.iter().find(|e| e.id == id).map(|e| &e.annotation)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut Annotation> {
        self.entries.iter_mut().find(|e| e.id == id).map(|e| &mut e.annotation)
    }

    /// Replaces the text contents of annotation `id`. Returns false for an unknown id.
    pub fn set_contents(&mut self, id: u32, contents: &str) -> bool {
        self.get_mut(id).map(|a| a.contents = contents.to_string()).is_some()
    }

    /// Moves annotation `id` so that its top-left corner is at `(x, y)`, keeping its size. Returns false for an unknown id.
    pub fn move_to(&mut self, id: u32, x: f32, y: f32) -> bool {
        self.get_mut(id)
            .map(|a| {
                a.bounds.x = x;
                a.bounds.y = y;
            })
            .is_some()
    }

    /// Sets the colour of annotation `id`, packed as the caller packs colours. Returns false for an unknown id.
    pub fn set_color(&mut self, id: u32, color: u32) -> bool {
        self.get_mut(id).map(|a| a.color = color).is_some()
    }

    /// The ids of the annotations of one kind, in stacking order, bottom first.
    pub fn ids_of(&self, subtype: Subtype) -> Vec<u32> {
        self.entries.iter().filter(|e| e.annotation.subtype == subtype).map(|e| e.id).collect()
    }

    /// The ids in stacking order, bottom first.
    pub fn ids(&self) -> Vec<u32> {
        self.entries.iter().map(|e| e.id).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn displays_annotations(&self) -> bool {
        self.displays
    }

    /// Shows or hides every annotation on the page. Hidden annotations stay on the page and cannot be hit.
    pub fn set_displays_annotations(&mut self, on: bool) {
        self.displays = on;
    }

    /// The id of the topmost annotation whose bounds contain the page point, or `None`. Nothing is hit while the
    /// annotations are hidden.
    pub fn annotation_at(&self, x: f32, y: f32) -> Option<u32> {
        if !self.displays {
            return None;
        }
        self.entries.iter().rev().find(|e| e.annotation.bounds.contains(x, y)).map(|e| e.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup_drawing::{Ink, InkKind, Stroke, StrokePoint};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn ids_count_up_from_one_and_are_never_reused() {
        let mut page = PageAnnotations::new();
        let a = page.add(Annotation::new(Subtype::Square, rect(0.0, 0.0, 10.0, 10.0)));
        let b = page.add(Annotation::new(Subtype::Text, rect(20.0, 0.0, 10.0, 10.0)));
        let c = page.add(Annotation::new(Subtype::Link, rect(40.0, 0.0, 10.0, 10.0)));
        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(page.ids(), vec![1, 2, 3]);
        assert_eq!(page.remove(2).map(|x| x.subtype), Some(Subtype::Text));
        assert_eq!(page.remove(2), None);
        assert_eq!(page.add(Annotation::new(Subtype::Stamp, rect(0.0, 0.0, 1.0, 1.0))), 4);
        assert_eq!(page.ids(), vec![1, 3, 4]);
    }

    #[test]
    fn the_topmost_annotation_under_a_point_is_hit() {
        let mut page = PageAnnotations::new();
        page.add(Annotation::new(Subtype::Square, rect(0.0, 0.0, 100.0, 100.0)));
        page.add(Annotation::new(Subtype::Stamp, rect(40.0, 40.0, 20.0, 20.0)));
        // the stamp sits above the square, and bounds are half open: the right edge at x = 100 is outside
        assert_eq!(page.annotation_at(50.0, 50.0), Some(2));
        assert_eq!(page.annotation_at(10.0, 10.0), Some(1));
        assert_eq!(page.annotation_at(100.0, 100.0), None);
    }

    #[test]
    fn removing_an_annotation_exposes_the_one_below_it() {
        let mut page = PageAnnotations::new();
        page.add(Annotation::new(Subtype::Square, rect(0.0, 0.0, 100.0, 100.0)));
        page.add(Annotation::new(Subtype::Stamp, rect(40.0, 40.0, 20.0, 20.0)));
        let removed = page.remove(2).unwrap();
        assert_eq!(removed.bounds, rect(40.0, 40.0, 20.0, 20.0));
        assert_eq!(page.annotation_at(50.0, 50.0), Some(1));
        assert_eq!(page.get(2), None);
        assert_eq!(page.len(), 1);
    }

    #[test]
    fn hidden_annotations_stay_on_the_page_but_cannot_be_hit() {
        let mut page = PageAnnotations::new();
        page.add(Annotation::new(Subtype::Square, rect(0.0, 0.0, 100.0, 100.0)));
        page.set_displays_annotations(false);
        assert!(!page.displays_annotations());
        assert_eq!(page.annotation_at(10.0, 10.0), None);
        assert_eq!(page.len(), 1);
        assert!(page.get(1).is_some());
        page.set_displays_annotations(true);
        assert_eq!(page.annotation_at(10.0, 10.0), Some(1));
    }

    #[test]
    fn an_ink_annotation_takes_the_bounds_of_its_drawing() {
        let ink = Ink { kind: InkKind::Pen, color: 0xff00_00ff, width: 4.0, opacity: 1.0 };
        let stroke = Stroke::new(ink, vec![StrokePoint::at(10.0, 10.0), StrokePoint::at(30.0, 20.0)]);
        let drawing = Drawing::from_strokes(vec![stroke]);
        let a = Annotation::ink(drawing.clone(), 0xffff_0000).unwrap();
        assert_eq!(a.subtype, Subtype::Ink);
        assert_eq!(a.bounds, rect(10.0, 10.0, 20.0, 10.0));
        assert_eq!(a.drawing, Some(drawing));
        assert_eq!(Annotation::ink(Drawing::new(), 0), None);
    }

    #[test]
    fn a_new_annotation_has_no_drawing_and_no_text() {
        let a = Annotation::new(Subtype::Highlight, rect(1.0, 2.0, 3.0, 4.0));
        assert_eq!(a.contents, "");
        assert_eq!(a.color, 0xff00_0000);
        assert!(a.drawing.is_none());
        assert!(PageAnnotations::default().is_empty());
    }
}
