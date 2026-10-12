//! PDF annotations as a pure model: the annotation kinds, the bounds of an annotation in page space, its display and
//! print flags, its text, author, border and highlight state, and the list of annotations a page holds (add, remove,
//! the topmost displayed annotation at a point, the annotations that print). The view draws what this model returns;
//! nothing here draws.

use crate::Rect;

/// The kinds of annotation a page can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subtype {
    Text,
    FreeText,
    Ink,
    Highlight,
    Underline,
    Strikeout,
    Squiggly,
    Square,
    Circle,
    Line,
    Stamp,
    Link,
    Widget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotError {
    /// The bounds are not finite, or have a negative width or height.
    BadBounds,
    /// The border width is not finite, or is negative.
    BadBorder,
}

fn valid_bounds(r: Rect) -> bool {
    r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite() && r.w >= 0.0 && r.h >= 0.0
}

/// One annotation. It shows on screen when `should_display` is set and prints when `should_print` is set.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    subtype: Subtype,
    bounds: Rect,
    contents: String,
    user_name: String,
    should_display: bool,
    should_print: bool,
    border_width: f32,
    highlighted: bool,
}

impl Annotation {
    /// An annotation of `subtype` covering `bounds` in page space. It displays and prints, has a border one point wide,
    /// and has no text or author.
    pub fn new(subtype: Subtype, bounds: Rect) -> Result<Annotation, AnnotError> {
        if !valid_bounds(bounds) {
            return Err(AnnotError::BadBounds);
        }
        Ok(Annotation {
            subtype,
            bounds,
            contents: String::new(),
            user_name: String::new(),
            should_display: true,
            should_print: true,
            border_width: 1.0,
            highlighted: false,
        })
    }

    pub fn subtype(&self) -> Subtype {
        self.subtype
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Moves or resizes the annotation. The bounds must be finite and must not have a negative size.
    pub fn set_bounds(&mut self, bounds: Rect) -> Result<(), AnnotError> {
        if !valid_bounds(bounds) {
            return Err(AnnotError::BadBounds);
        }
        self.bounds = bounds;
        Ok(())
    }

    /// The textual content of the annotation, such as the text of a note.
    pub fn contents(&self) -> &str {
        &self.contents
    }

    pub fn set_contents(&mut self, text: &str) {
        self.contents = text.to_string();
    }

    /// The name of the user who made the annotation.
    pub fn user_name(&self) -> &str {
        &self.user_name
    }

    pub fn set_user_name(&mut self, name: &str) {
        self.user_name = name.to_string();
    }

    pub fn should_display(&self) -> bool {
        self.should_display
    }

    pub fn set_should_display(&mut self, on: bool) {
        self.should_display = on;
    }

    pub fn should_print(&self) -> bool {
        self.should_print
    }

    pub fn set_should_print(&mut self, on: bool) {
        self.should_print = on;
    }

    /// The width of the border in points.
    pub fn border_width(&self) -> f32 {
        self.border_width
    }

    /// Sets the border width. It must be finite and not negative.
    pub fn set_border_width(&mut self, width: f32) -> Result<(), AnnotError> {
        if !width.is_finite() || width < 0.0 {
            return Err(AnnotError::BadBorder);
        }
        self.border_width = width;
        Ok(())
    }

    /// Whether the annotation is in a highlighted state, such as while a link is pressed.
    pub fn is_highlighted(&self) -> bool {
        self.highlighted
    }

    pub fn set_highlighted(&mut self, on: bool) {
        self.highlighted = on;
    }

    /// Whether the point `(x, y)` in page space is inside the annotation. The left and bottom edges are inside and the
    /// right and top edges are outside, as for any rectangle here.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.bounds.contains(x, y)
    }
}

/// The annotations of one page, in drawing order: the last one added is on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageAnnotations {
    items: Vec<Annotation>,
}

impl PageAnnotations {
    pub fn new() -> PageAnnotations {
        PageAnnotations { items: Vec::new() }
    }

    /// Adds an annotation on top of the others.
    pub fn add(&mut self, annotation: Annotation) {
        self.items.push(annotation);
    }

    /// Removes the annotation at `index` in drawing order and returns it, or `None` when there is none at that index.
    pub fn remove(&mut self, index: usize) -> Option<Annotation> {
        if index < self.items.len() {
            Some(self.items.remove(index))
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The annotation at `index` in drawing order.
    pub fn get(&self, index: usize) -> Option<&Annotation> {
        self.items.get(index)
    }

    /// All annotations, in drawing order.
    pub fn annotations(&self) -> &[Annotation] {
        &self.items
    }

    /// The index of the topmost annotation that displays and contains the point `(x, y)`. Hidden annotations are not
    /// hit, so a point over a hidden annotation finds the one beneath it.
    pub fn annotation_at(&self, x: f32, y: f32) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .rev()
            .find(|(_, a)| a.should_display && a.contains(x, y))
            .map(|(i, _)| i)
    }

    /// The annotations that print, in drawing order.
    pub fn printed(&self) -> Vec<&Annotation> {
        self.items.iter().filter(|a| a.should_print).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn bounds_with_a_negative_size_or_a_non_finite_edge_are_refused() {
        assert_eq!(Annotation::new(Subtype::Square, at(0.0, 0.0, -1.0, 5.0)), Err(AnnotError::BadBounds));
        assert_eq!(Annotation::new(Subtype::Square, at(f32::NAN, 0.0, 5.0, 5.0)), Err(AnnotError::BadBounds));
        let mut a = Annotation::new(Subtype::Circle, at(0.0, 0.0, 4.0, 4.0)).unwrap();
        assert_eq!(a.set_bounds(at(0.0, 0.0, 4.0, -2.0)), Err(AnnotError::BadBounds));
        assert_eq!(a.bounds(), at(0.0, 0.0, 4.0, 4.0));
    }

    #[test]
    fn a_new_annotation_displays_and_prints_with_a_one_point_border() {
        let a = Annotation::new(Subtype::Highlight, at(10.0, 10.0, 80.0, 12.0)).unwrap();
        assert!(a.should_display());
        assert!(a.should_print());
        assert_eq!(a.border_width(), 1.0);
        assert_eq!(a.contents(), "");
        assert!(!a.is_highlighted());
    }

    #[test]
    fn border_width_must_be_finite_and_not_negative() {
        let mut a = Annotation::new(Subtype::Ink, at(0.0, 0.0, 9.0, 9.0)).unwrap();
        assert_eq!(a.set_border_width(-1.0), Err(AnnotError::BadBorder));
        assert_eq!(a.set_border_width(f32::INFINITY), Err(AnnotError::BadBorder));
        assert_eq!(a.border_width(), 1.0);
        a.set_border_width(2.5).unwrap();
        assert_eq!(a.border_width(), 2.5);
    }

    #[test]
    fn add_and_remove_work_by_drawing_order() {
        let mut page = PageAnnotations::new();
        page.add(Annotation::new(Subtype::Text, at(0.0, 0.0, 5.0, 5.0)).unwrap());
        page.add(Annotation::new(Subtype::Stamp, at(20.0, 0.0, 5.0, 5.0)).unwrap());
        assert_eq!(page.len(), 2);
        let first = page.remove(0).unwrap();
        assert_eq!(first.subtype(), Subtype::Text);
        assert_eq!(page.len(), 1);
        assert_eq!(page.get(0).unwrap().subtype(), Subtype::Stamp);
        assert!(page.remove(5).is_none());
    }

    #[test]
    fn point_hit_test_returns_the_topmost_annotation() {
        let mut page = PageAnnotations::new();
        page.add(Annotation::new(Subtype::Square, at(0.0, 0.0, 10.0, 10.0)).unwrap());
        page.add(Annotation::new(Subtype::Circle, at(5.0, 5.0, 10.0, 10.0)).unwrap());
        assert_eq!(page.annotation_at(6.0, 6.0), Some(1));
        assert_eq!(page.annotation_at(1.0, 1.0), Some(0));
        assert_eq!(page.annotation_at(20.0, 20.0), None);
    }

    #[test]
    fn hidden_annotations_are_not_hit() {
        let mut page = PageAnnotations::new();
        page.add(Annotation::new(Subtype::Square, at(0.0, 0.0, 10.0, 10.0)).unwrap());
        let mut top = Annotation::new(Subtype::Circle, at(5.0, 5.0, 10.0, 10.0)).unwrap();
        top.set_should_display(false);
        page.add(top);
        assert_eq!(page.annotation_at(6.0, 6.0), Some(0));
    }

    #[test]
    fn hit_test_treats_the_right_and_top_edges_as_outside() {
        let mut page = PageAnnotations::new();
        page.add(Annotation::new(Subtype::Underline, at(0.0, 0.0, 10.0, 10.0)).unwrap());
        assert_eq!(page.annotation_at(0.0, 0.0), Some(0));
        assert_eq!(page.annotation_at(10.0, 5.0), None);
        assert_eq!(page.annotation_at(5.0, 10.0), None);
    }

    #[test]
    fn only_annotations_that_print_are_listed_for_printing() {
        let mut page = PageAnnotations::new();
        let mut hidden_on_paper = Annotation::new(Subtype::Link, at(0.0, 0.0, 4.0, 4.0)).unwrap();
        hidden_on_paper.set_should_print(false);
        page.add(hidden_on_paper);
        page.add(Annotation::new(Subtype::Strikeout, at(0.0, 0.0, 4.0, 4.0)).unwrap());
        let printed = page.printed();
        assert_eq!(printed.len(), 1);
        assert_eq!(printed[0].subtype(), Subtype::Strikeout);
    }

    #[test]
    fn contents_and_author_are_stored_as_given() {
        let mut a = Annotation::new(Subtype::FreeText, at(0.0, 0.0, 50.0, 20.0)).unwrap();
        a.set_contents("Check the total");
        a.set_user_name("Reviewer");
        assert_eq!(a.contents(), "Check the total");
        assert_eq!(a.user_name(), "Reviewer");
    }
}
