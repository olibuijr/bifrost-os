//! Article pictures: where each feed keeps them, fetching on a worker and
//! scaling to the size a row or the article page draws them at.
//! RUV names a webp rendition through a base64 JSON recipe (rewritten to a
//! JPEG of the drawn size), mbl.is puts an <img> in the description, and
//! Visir carries none in the feed, so its article page's og:image is used.

use springboard::gfx::{pack, Image};
use springboard::shell::music::radio::Fetch;

const PAGE_LIMIT: usize = 1_500_000;
const IMAGE_LIMIT: usize = 4_000_000;

/// Fetches the picture for an article (`image` from the feed, else the page's og:image) as `w` x `h`,
/// cropped to fill, with `r` px rounded corners over `bg`. None when there is none or it fails.
pub fn load(fetcher: &dyn Fetch, image: &str, link: &str, w: usize, h: usize, r: usize, bg: u32) -> Option<Image> {
    let url = if image.is_empty() {
        let page = fetcher.get(link, PAGE_LIMIT).ok()?;
        og_image(&String::from_utf8_lossy(&page))?
    } else {
        image.to_string()
    };
    let url = ruv_jpeg(&url, w, h).unwrap_or(url);
    let bytes = fetcher.get(&url, IMAGE_LIMIT).ok()?;
    let img = springboard::imgdec::decode(&bytes)?;
    let mut out = springboard::imgdec::cover(&img, w, h);
    round(&mut out, r, bg);
    Some(out)
}

/// `<meta property="og:image" content="...">`, attributes in either order.
pub fn og_image(html: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = html[from..].find("<meta") {
        let at = from + i;
        let end = html[at..].find('>').map_or(html.len(), |e| at + e);
        let tag = &html[at..end];
        if tag.contains("og:image\"") || tag.contains("og:image'") {
            let c = tag.find("content=")? + 8;
            let q = tag[c..].chars().next()?;
            let rest = &tag[c + 1..];
            return rest.find(q).map(|e| crate::feed::decode(&rest[..e])).filter(|u| u.starts_with("http"));
        }
        from = end;
    }
    None
}

/// RUV's image server takes a base64 JSON recipe; asks it for a JPEG at the size drawn
/// (the decoder has no webp). None when `url` is not one of those.
pub fn ruv_jpeg(url: &str, w: usize, h: usize) -> Option<String> {
    let recipe = url.strip_prefix("https://myndir.ruv.is/")?;
    let json = String::from_utf8(b64_decode(recipe)?).ok()?;
    let key = json.split("\"key\"").nth(1)?.split('"').nth(1)?;
    let bucket = json.split("\"bucket\"").nth(1)?.split('"').nth(1)?;
    let edit = format!("{{\"bucket\": \"{bucket}\", \"key\": \"{key}\", \"edits\": {{\"toFormat\": \"jpeg\", \"jpeg\": {{\"quality\": 70}}, \"resize\": {{\"width\": {w}, \"fit\": \"cover\", \"height\": {h}}}}}}}");
    Some(format!("https://myndir.ruv.is/{}", b64_encode(edit.as_bytes())))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { B64[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = vec![];
    let (mut acc, mut bits) = (0u32, 0);
    for b in s.bytes().filter(|&b| b != b'=') {
        let v = B64.iter().position(|&c| c == b)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Paints the corners outside a radius-`r` arc with `bg` (anti-aliased).
pub fn round(img: &mut Image, r: usize, bg: u32) {
    let (w, h) = (img.w, img.h);
    if r == 0 || w < 2 * r || h < 2 * r {
        return;
    }
    let (br, bgc, bb) = (bg & 0xff, bg >> 8 & 0xff, bg >> 16 & 0xff);
    for dy in 0..r {
        for dx in 0..r {
            let (fx, fy) = (r as f32 - dx as f32 - 0.5, r as f32 - dy as f32 - 0.5);
            let cover = (r as f32 - (fx * fx + fy * fy).sqrt() + 0.5).clamp(0.0, 1.0);
            if cover >= 1.0 {
                continue;
            }
            for (x, y) in [(dx, dy), (w - 1 - dx, dy), (dx, h - 1 - dy), (w - 1 - dx, h - 1 - dy)] {
                let p = &mut img.px[y * w + x];
                let mix = |a: u32, b: u32| (a as f32 * cover + b as f32 * (1.0 - cover)).round() as u8;
                *p = pack(mix(*p & 0xff, br), mix(*p >> 8 & 0xff, bgc), mix(*p >> 16 & 0xff, bb), 255);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruv_recipes_become_jpegs_of_the_drawn_size() {
        let src = format!("https://myndir.ruv.is/{}", b64_encode(br#"{"bucket": "b", "key": "media/x.jpg", "edits": {"toFormat": "webp"}}"#));
        let out = ruv_jpeg(&src, 240, 160).unwrap();
        let json = String::from_utf8(b64_decode(out.strip_prefix("https://myndir.ruv.is/").unwrap()).unwrap()).unwrap();
        assert!(json.contains("\"key\": \"media/x.jpg\"") && json.contains("\"toFormat\": \"jpeg\"") && json.contains("\"width\": 240"));
        assert!(ruv_jpeg("https://c.arvakur.is/a.jpg", 1, 1).is_none());
    }

    #[test]
    fn base64_round_trips() {
        for s in ["", "a", "ab", "abc", "abcd", "{\"k\": 1}"] {
            assert_eq!(b64_decode(&b64_encode(s.as_bytes())).unwrap(), s.as_bytes());
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn og_image_reads_the_meta_tag() {
        let html = r#"<head><meta name="x" content="y"><meta property="og:image" content="https://www.visir.is/i/A_1600x900.jpg" /></head>"#;
        assert_eq!(og_image(html).as_deref(), Some("https://www.visir.is/i/A_1600x900.jpg"));
        assert_eq!(og_image("<meta property=\"og:title\" content=\"t\">"), None);
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -p news -- --ignored live`: loads one picture per default feed over the network.
    #[test]
    #[ignore]
    fn live_feeds_have_pictures() {
        let http = springboard::shell::music::radio::Http::default();
        for (name, url) in crate::news::DEFAULT_FEEDS {
            let xml = springboard::shell::music::radio::Fetch::get(&http, url, 2_000_000).unwrap();
            let items = crate::feed::parse(&String::from_utf8_lossy(&xml), name);
            let it = &items[0];
            let img = super::load(&http, &it.image, &it.link, 240, 160, 12, 0);
            println!("{name}: image={:?} -> {:?}", it.image.chars().take(60).collect::<String>(), img.as_ref().map(|i| (i.w, i.h)));
            assert!(img.is_some(), "{name}");
        }
    }
}
