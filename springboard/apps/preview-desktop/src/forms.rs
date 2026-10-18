//! The fillable fields of a PDF form: text fields and check boxes. The worker reads them when the file opens, with each
//! widget's page and rectangle, and the document keeps their values in the kit's `acroform::FieldSet`. The values are
//! written back by the saver (see `Document::field_edits`). Radio buttons, push buttons, choice lists and signatures
//! are not filled here.

use crate::model::Norm;
use bifrost_app::controls::acroform::{Field, FieldSet};
use springboard::cg::pdf::doc::Doc;
use springboard::cg::pdf::obj::{Dict, Obj};
use springboard::cg::pdf::page::{self, Page as PdfPage, PageBox};
use std::collections::HashMap;

/// The field flag bits of a radio button and of a push button (bits 16 and 17, counted from 1).
const RADIO: i64 = 1 << 15;
const PUSH_BUTTON: i64 = 1 << 16;

/// The kinds of field that are filled here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Check,
}

/// A widget of a filled field: its source page, its rectangle in normalised, unrotated page coordinates, the field it
/// holds a value for, and the objects its value is written into.
#[derive(Clone, Debug, PartialEq)]
pub struct FormWidget {
    pub page: usize,
    pub rect: Norm,
    /// The fully qualified field name, the key of the field in the form's `FieldSet`.
    pub name: String,
    pub kind: Kind,
    /// The widget's object number (0 when the widget is not an object of its own).
    pub obj: u32,
    /// The object number of the field dictionary that holds /V (0 when it is not an object of its own).
    pub node: u32,
    /// A check box's on state, from its appearance. Empty for a text field.
    pub on_name: String,
}

/// The fillable fields of a form, with their values, and each widget of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Form {
    pub fields: FieldSet,
    pub widgets: Vec<FormWidget>,
}

/// The fillable fields of `doc`, placed on its `pages`. A file with no form gives an empty one.
pub fn detect(doc: &Doc, pages: &[PdfPage]) -> Form {
    let mut form = Form::default();
    let Some(catalog) = doc.catalog() else { return form };
    let Obj::Dict(acro) = doc.field(&catalog, "AcroForm") else { return form };
    let Obj::Array(roots) = doc.field(&acro, "Fields") else { return form };
    let walk = Walk { doc, pages, on_page: widget_pages(doc, pages) };
    for root in &roots {
        if let Obj::Dict(d) = doc.get(root) {
            walk.visit(num_of(root), &d, "", &Inherited::default(), &mut form);
        }
    }
    form
}

/// What a field inherits from its parents in the field tree: its type, flags and value.
#[derive(Clone, Default)]
struct Inherited {
    ft: String,
    ff: i64,
    value: Option<Obj>,
}

struct Walk<'a> {
    doc: &'a Doc,
    pages: &'a [PdfPage],
    /// The page index of each widget, by its object number.
    on_page: HashMap<u32, usize>,
}

impl Walk<'_> {
    /// Visits the field `d` (object `num`, 0 when direct) under the partial name `parent`.
    fn visit(&self, num: u32, d: &Dict, parent: &str, inh: &Inherited, form: &mut Form) {
        let doc = self.doc;
        let name = match doc.field(d, "T") {
            Obj::Str(bytes) => join(parent, &page::text_string(&bytes)),
            _ => parent.to_string(),
        };
        let ft = doc.field(d, "FT").name().map(str::to_string).unwrap_or_else(|| inh.ft.clone());
        let ff = doc.field(d, "Ff").int().unwrap_or(inh.ff);
        let value = match d.get("V") {
            Some(v) => doc.get(v),
            None => inh.value.clone().unwrap_or(Obj::Null),
        };
        let mine = Inherited { ft, ff, value: Some(value) };
        let kids = match doc.field(d, "Kids") {
            Obj::Array(items) => items,
            _ => Vec::new(),
        };
        if kids.is_empty() {
            // a field whose only widget is the field itself
            self.widget(num, num, d, &name, &mine, form);
            return;
        }
        for kid in &kids {
            let Obj::Dict(kd) = doc.get(kid) else { continue };
            if kd.contains_key("T") || kd.contains_key("Kids") {
                self.visit(num_of(kid), &kd, &name, &mine, form);
            } else {
                // a widget of this field: its value sits on this node
                self.widget(num_of(kid), num, &kd, &name, &mine, form);
            }
        }
    }

    /// Adds the widget `d` (object `obj`) of field `name`, whose value is on `node`, when it is a fillable kind.
    fn widget(&self, obj: u32, node: u32, d: &Dict, name: &str, inh: &Inherited, form: &mut Form) {
        let Some(&page) = self.on_page.get(&obj) else { return };
        let kind = match inh.ft.as_str() {
            "Tx" => Kind::Text,
            "Btn" if inh.ff & (RADIO | PUSH_BUTTON) == 0 => Kind::Check,
            _ => return,
        };
        let Some(rect) = self.rect(d, page) else { return };
        let on_name = self.on_state(d);
        if form.fields.get(name).is_none() {
            let field = match kind {
                Kind::Text => Field::text(name, inh.ff as u32).with_default(&text_of(inh.value.as_ref()), false),
                Kind::Check => {
                    let on = matches!(&inh.value, Some(Obj::Name(n)) if n.as_str() != "Off");
                    Field::button(name, inh.ff as u32).with_default("", on)
                }
            };
            let _ = form.fields.add(field);
        }
        form.widgets.push(FormWidget { page, rect, name: name.to_string(), kind, obj, node, on_name });
    }

    /// The widget's rectangle in normalised, unrotated page coordinates. The same mapping the markup uses.
    fn rect(&self, d: &Dict, page: usize) -> Option<Norm> {
        let Obj::Array(corners) = self.doc.field(d, "Rect") else { return None };
        let values: Vec<f64> = corners.iter().filter_map(|o| self.doc.get(o).num()).collect();
        let &[x0, y0, x1, y1] = values.as_slice() else { return None };
        let m = self.pages.get(page)?.transform(PageBox::Crop, (0.0, 0.0, 1.0, 1.0), false);
        let a = page::apply(&m, x0, y0);
        let b = page::apply(&m, x1, y1);
        Some(Norm::from_corners((a.0 as f32, a.1 as f32), (b.0 as f32, b.1 as f32)))
    }

    /// A check box's on state: its appearance's first state other than Off.
    fn on_state(&self, d: &Dict) -> String {
        if let Obj::Dict(ap) = self.doc.field(d, "AP") {
            if let Obj::Dict(states) = self.doc.field(&ap, "N") {
                let mut names: Vec<String> = states.keys().filter(|k| k.as_str() != "Off").cloned().collect();
                names.sort();
                if let Some(first) = names.into_iter().next() {
                    return first;
                }
            }
        }
        "Yes".into()
    }
}

/// The page index of every widget of the document, by object number. Widgets that are not on a page have no entry.
fn widget_pages(doc: &Doc, pages: &[PdfPage]) -> HashMap<u32, usize> {
    let mut map = HashMap::new();
    for (i, p) in pages.iter().enumerate() {
        if let Obj::Array(annots) = doc.field(&p.dict, "Annots") {
            for a in &annots {
                if let Obj::Ref(n, _) = a {
                    map.insert(*n, i);
                }
            }
        }
    }
    map
}

/// The object number of `o` when it is a reference, 0 when it is a direct object.
fn num_of(o: &Obj) -> u32 {
    match o {
        Obj::Ref(n, _) => *n,
        _ => 0,
    }
}

fn join(parent: &str, partial: &str) -> String {
    if parent.is_empty() {
        partial.to_string()
    } else {
        format!("{parent}.{partial}")
    }
}

/// A field's text value: a text string, or a name's text.
fn text_of(value: Option<&Obj>) -> String {
    match value {
        Some(Obj::Str(bytes)) => page::text_string(bytes),
        Some(Obj::Name(n)) => n.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with objects 1 to n (`objects[n - 1]`), a classic cross-reference table and a trailer with /Root 1 0 R.
    fn file(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = vec![];
        for (i, o) in objects.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in &offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objects.len() + 1, xref).as_bytes());
        out
    }

    const FORM: [&str; 5] = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >>",
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Ada) /Rect [0 0 100 50] /P 3 0 R >>",
        "<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /V /Yes /AS /Yes /AP << /N << /Yes null /Off null >> >> /Rect [100 50 200 100] /P 3 0 R >>",
    ];

    #[test]
    fn text_fields_and_check_boxes_are_found_with_their_values_pages_and_places() {
        let doc = Doc::open(file(&FORM)).unwrap();
        let pages = page::pages(&doc);
        let form = detect(&doc, &pages);
        assert_eq!(form.fields.len(), 2);
        assert_eq!(form.fields.get("name").expect("the text field").value(), "Ada");
        assert!(form.fields.get("agree").expect("the check box").is_on());
        assert_eq!(form.widgets.len(), 2);
        let text = &form.widgets[0];
        assert_eq!((text.page, text.kind, text.node, text.obj), (0, Kind::Text, 4, 4));
        assert!(text.rect.x.abs() < 1e-4 && (text.rect.w - 0.5).abs() < 1e-4, "{:?}", text.rect);
        let check = &form.widgets[1];
        assert_eq!((check.kind, check.on_name.as_str()), (Kind::Check, "Yes"));
    }

    #[test]
    fn a_file_without_a_form_has_no_fields() {
        let doc = Doc::open(file(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"])).unwrap();
        let pages = page::pages(&doc);
        assert_eq!(detect(&doc, &pages), Form::default());
    }

    #[test]
    fn a_field_takes_its_name_from_its_parents() {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>",
            "<< /T (address) /Kids [5 0 R] >>",
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (street) /Rect [0 0 100 50] /P 3 0 R >>",
        ];
        let doc = Doc::open(file(&objects)).unwrap();
        let pages = page::pages(&doc);
        let form = detect(&doc, &pages);
        assert!(form.fields.get("address.street").is_some());
        assert_eq!(form.widgets[0].node, 5, "the value sits on the terminal field, which has the name");
    }
}
