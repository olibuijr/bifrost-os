//! The user-agent style sheet: HTML default rendering in the spirit of the
//! WHATWG rendering section, with modern defaults for form controls.

use super::parse::{parse_stylesheet, Origin, Stylesheet};

/// The user-agent style sheet source.
pub const UA_CSS: &str = r#"
html, body, address, article, aside, blockquote, center, dd, details, dialog, div, dl, dt,
fieldset, figcaption, figure, footer, form, h1, h2, h3, h4, h5, h6, header, hgroup, hr,
legend, main, menu, nav, ol, optgroup, p, pre, search, section, summary, ul, xmp, listing,
plaintext { display: block; }

head, link, meta, script, style, title, base, template, noscript, area, datalist, param, source,
track, rp, [hidden]:not([hidden="until-found" i]), dialog:not([open]),
input[type="hidden" i] { display: none; }

details:not([open]) > :not(summary) { display: none; }

li { display: list-item; }
table { display: table; box-sizing: border-box; border-spacing: 2px; }
caption { display: table-caption; text-align: center; }
colgroup { display: table-column-group; }
col { display: table-column; }
thead { display: table-header-group; }
tbody { display: table-row-group; }
tfoot { display: table-footer-group; }
tr { display: table-row; }
td, th { display: table-cell; padding: 1px; vertical-align: inherit; }
th { font-weight: bold; text-align: center; }
input, button, select, textarea, meter, progress, img, video, canvas, audio, svg,
iframe, embed, object { display: inline-block; }
img, video, canvas, svg { display: inline; }

body { margin: 8px; }
p, dl, menu, ol, ul { margin-top: 1em; margin-bottom: 1em; }
blockquote, figure { margin: 1em 40px; }
dd { margin-left: 40px; }
ol, ul, menu { padding-left: 40px; }
ul { list-style-type: disc; }
ol { list-style-type: decimal; }
ul ul, ol ul { list-style-type: circle; }
ul ul ul, ol ul ul, ul ol ul, ol ol ul { list-style-type: square; }
ul ol, ol ol { list-style-type: decimal; }
dir, menu { list-style-type: disc; }

h1 { font-size: 2em; margin-top: 0.67em; margin-bottom: 0.67em; }
h2 { font-size: 1.5em; margin-top: 0.83em; margin-bottom: 0.83em; }
h3 { font-size: 1.17em; margin-top: 1em; margin-bottom: 1em; }
h4 { font-size: 1em; margin-top: 1.33em; margin-bottom: 1.33em; }
h5 { font-size: 0.83em; margin-top: 1.67em; margin-bottom: 1.67em; }
h6 { font-size: 0.67em; margin-top: 2.33em; margin-bottom: 2.33em; }
h1, h2, h3, h4, h5, h6 { font-weight: bold; }

b, strong { font-weight: bolder; }
i, em, cite, var, dfn, address { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
abbr[title], acronym[title] { text-decoration: underline dotted; }
small { font-size: smaller; }
big { font-size: larger; }
sub { vertical-align: sub; font-size: smaller; }
sup { vertical-align: super; font-size: smaller; }
mark { background-color: yellow; color: black; }
q::before { content: open-quote; }
q::after { content: close-quote; }
center { text-align: center; }

pre, xmp, listing, plaintext { white-space: pre; margin-top: 1em; margin-bottom: 1em; }
code, kbd, samp, tt, pre, xmp, listing, plaintext { font-family: monospace; }
hr { margin: 0.5em auto; border: 1px inset; color: gray; overflow: hidden; }

a:link { color: #0000ee; text-decoration: underline; }
a:visited { color: #551a8b; text-decoration: underline; }

fieldset { margin: 0 2px; padding: 0.35em 0.75em 0.625em; border: 2px groove; }
legend { padding: 0 2px; }
input, button, select, textarea {
  font-family: sans-serif; font-size: 13.3333px; color: black; margin: 0;
  text-align: start; box-sizing: border-box;
}
input, textarea, select { background-color: white; border: 2px inset; padding: 1px 2px; }
button, input[type="button" i], input[type="submit" i], input[type="reset" i] {
  background-color: #efefef; border: 2px outset #767676; padding: 1px 6px;
  text-align: center; border-radius: 3px;
}
input[type="checkbox" i], input[type="radio" i] { padding: 0; margin: 3px 3px 3px 4px; }
textarea { white-space: pre-wrap; font-family: monospace; }
select { border-radius: 0; }
progress, meter { vertical-align: -0.2em; }
summary { display: block; }
iframe { border: 2px inset; }
video { object-fit: contain; }
"#;

/// The parsed user-agent style sheet.
pub fn ua_stylesheet() -> Stylesheet {
    parse_stylesheet(UA_CSS, Origin::UserAgent)
}

#[cfg(test)]
mod tests {
    use super::super::super::dom::{parse_html, Dom};
    use super::super::cascade::{StyleEngine, StyleTree};
    use super::super::computed::Computed;
    use super::super::parse::MediaEnv;
    use super::super::values::*;
    use super::*;

    fn styled(body: &str) -> (Dom, StyleTree) {
        let dom = parse_html(&format!("<html><head><title>t</title></head><body>{}</body></html>", body));
        let engine = StyleEngine::new(vec![ua_stylesheet()], MediaEnv { width: 800.0, height: 600.0, dark: false });
        let tree = engine.compute(&dom);
        (dom, tree)
    }

    fn of<'a>(dom: &Dom, tree: &'a StyleTree, id: &str) -> &'a Computed {
        tree.get(dom.get_element_by_id(id).unwrap_or_else(|| panic!("no #{}", id))).unwrap()
    }

    #[test]
    fn parses_without_dropping_many_rules() {
        let sheet = ua_stylesheet();
        assert!(sheet.rules().len() > 100, "only {} rules", sheet.rules().len());
        // Every `{` block in the source produced a rule.
        let blocks = UA_CSS.matches('{').count();
        assert!(sheet.rules().len() >= blocks);
    }

    #[test]
    fn blocks_and_inlines() {
        let (dom, tree) = styled("<div id=d><span id=s>x</span><p id=p>y</p><ul id=u><li id=l>z</li></ul></div>");
        assert_eq!(of(&dom, &tree, "d").display, Display::Block);
        assert_eq!(of(&dom, &tree, "s").display, Display::Inline);
        assert_eq!(of(&dom, &tree, "p").display, Display::Block);
        assert_eq!(of(&dom, &tree, "l").display, Display::ListItem);
        let head = tree.get(dom.head().unwrap()).unwrap();
        assert_eq!(head.display, Display::None);
        let title = tree.get(dom.elements_by_tag("title")[0]).unwrap();
        assert_eq!(title.display, Display::None);
        assert_eq!(tree.get(dom.body().unwrap()).unwrap().display, Display::Block);
        assert_eq!(tree.get(dom.document_element().unwrap()).unwrap().display, Display::Block);
    }

    #[test]
    fn default_margins_and_padding() {
        let (dom, tree) = styled("<p id=p>y</p><ul id=u><li>z</li></ul><blockquote id=b>q</blockquote>");
        assert_eq!(tree.get(dom.body().unwrap()).unwrap().margin_top, Length::Px(8.0));
        assert_eq!(of(&dom, &tree, "p").margin_top, Length::Px(16.0));
        assert_eq!(of(&dom, &tree, "u").padding_left, Length::Px(40.0));
        let b = of(&dom, &tree, "b");
        assert_eq!(b.margin_left, Length::Px(40.0));
        assert_eq!(b.margin_top, Length::Px(16.0));
    }

    #[test]
    fn headings_scale_and_bold() {
        let (dom, tree) = styled("<h1 id=a>x</h1><h2 id=b>x</h2><h6 id=c>x</h6>");
        let a = of(&dom, &tree, "a");
        assert_eq!(a.font_size, 32.0);
        assert_eq!(a.font_weight, 700.0);
        assert!((a.margin_top.resolve(None).unwrap() - 21.44).abs() < 0.01);
        assert_eq!(of(&dom, &tree, "b").font_size, 24.0);
        assert!((of(&dom, &tree, "c").font_size - 10.72).abs() < 0.01);
    }

    #[test]
    fn inline_text_semantics() {
        let (dom, tree) = styled("<b id=b>x</b><i id=i>x</i><u id=u>x</u><s id=s>x</s><code id=c>x</code><small id=sm>x</small>");
        assert_eq!(of(&dom, &tree, "b").font_weight, 700.0);
        assert_eq!(of(&dom, &tree, "i").font_style, FontStyle::Italic);
        assert!(of(&dom, &tree, "u").text_decoration_line.underline);
        assert!(of(&dom, &tree, "s").text_decoration_line.line_through);
        assert_eq!(of(&dom, &tree, "c").font_family, vec!["monospace".to_string()]);
        assert!(of(&dom, &tree, "sm").font_size < 16.0);
    }

    #[test]
    fn links_and_lists() {
        let (dom, tree) = styled("<a id=a href='/x'>x</a><a id=n>y</a><ul><li><ul><li><ul id=deep><li>z</li></ul></li></ul></li></ul><ol id=o></ol>");
        let a = of(&dom, &tree, "a");
        assert_eq!(a.color, Rgba::from_u8(0, 0, 238, 1.0));
        assert!(a.text_decoration_line.underline);
        let n = of(&dom, &tree, "n");
        assert_eq!(n.color, Rgba::BLACK);
        assert!(!n.text_decoration_line.underline);
        assert_eq!(of(&dom, &tree, "deep").list_style_type, ListStyleType::Square);
        assert_eq!(of(&dom, &tree, "o").list_style_type, ListStyleType::Decimal);
    }

    #[test]
    fn hidden_elements_and_details() {
        let (dom, tree) = styled(
            "<div id=h hidden>x</div><details id=d><summary id=s>s</summary><p id=in>in</p></details><details open><p id=op>x</p></details><input id=hi type=HIDDEN><dialog id=dl>x</dialog>",
        );
        assert_eq!(of(&dom, &tree, "h").display, Display::None);
        assert_eq!(of(&dom, &tree, "in").display, Display::None);
        assert_eq!(of(&dom, &tree, "s").display, Display::Block);
        assert_eq!(of(&dom, &tree, "op").display, Display::Block);
        assert_eq!(of(&dom, &tree, "hi").display, Display::None);
        assert_eq!(of(&dom, &tree, "dl").display, Display::None);
    }

    #[test]
    fn tables_and_controls() {
        let (dom, tree) = styled("<table id=t><tr id=r><th id=h>a</th><td id=c>b</td></tr></table><input id=i><button id=b>x</button><textarea id=ta></textarea>");
        assert_eq!(of(&dom, &tree, "t").display, Display::Table);
        assert_eq!(of(&dom, &tree, "r").display, Display::TableRow);
        assert_eq!(of(&dom, &tree, "c").display, Display::TableCell);
        assert_eq!(of(&dom, &tree, "h").font_weight, 700.0);
        assert_eq!(of(&dom, &tree, "i").display, Display::InlineBlock);
        assert_eq!(of(&dom, &tree, "b").display, Display::InlineBlock);
        assert_eq!(of(&dom, &tree, "b").text_align, TextAlign::Center);
        assert_eq!(of(&dom, &tree, "ta").white_space, WhiteSpace::PreWrap);
        assert_eq!(of(&dom, &tree, "i").box_sizing, BoxSizing::BorderBox);
    }

    #[test]
    fn pre_and_quotes() {
        let (dom, tree) = styled("<pre id=p>x</pre><q id=q>x</q>");
        assert_eq!(of(&dom, &tree, "p").white_space, WhiteSpace::Pre);
        let q = dom.get_element_by_id("q").unwrap();
        assert!(tree.before(q).is_some());
        assert!(tree.after(q).is_some());
    }
}
