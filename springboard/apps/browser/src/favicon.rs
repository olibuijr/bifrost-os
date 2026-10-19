//! Cache and draw favicons supplied by Servo's native favicon callback.
//! Discovery, authenticated resource fetching and image decoding belong to Servo.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use bifrost_app::kit::{rgba, Image};
use bifrost_app::webview::WebViewId;

pub const STORE_PX: usize = 32;
pub const ICON_PT: f32 = 16.0;
pub const GAP_PT: f32 = 6.0;
const MAX_MEMORY_ICONS: usize = 256;

struct Icon {
    image: Image,
    png: Vec<u8>,
    scaled: Option<(usize, Image)>,
}

pub struct Favicons {
    dir: Option<PathBuf>,
    cache: HashMap<String, Option<Icon>>,
    order: VecDeque<String>,
    views: HashMap<WebViewId, String>,
}

impl Default for Favicons {
    fn default() -> Self {
        Self::new(Some(crate::store::data_dir().join("favicons")))
    }
}

pub fn host_key(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    Some(url[url::Position::BeforeHost..url::Position::AfterPort].to_string())
}

fn file_stem(host: &str) -> String {
    // Encoding is injective: different origins cannot share a cache file through punctuation replacement.
    bifrost_core::plist::hex_encode(host.as_bytes())
}

impl Favicons {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir, cache: HashMap::new(), order: VecDeque::new(), views: HashMap::new() }
    }

    pub fn set_private(&mut self, private: bool) {
        if private {
            self.dir = None;
        }
    }

    fn insert(&mut self, host: String, icon: Option<Icon>) {
        if !self.cache.contains_key(&host) {
            self.order.push_back(host.clone());
            if self.order.len() > MAX_MEMORY_ICONS {
                if let Some(old) = self.order.pop_front() {
                    self.cache.remove(&old);
                }
            }
        }
        self.cache.insert(host, icon);
    }

    pub fn on_native(&mut self, view: WebViewId, page_url: &str, pixels: &[u8]) {
        if pixels.len() != STORE_PX * STORE_PX * 4 {
            return;
        }
        let Some(host) = host_key(page_url) else { return };
        let png = encode_png(pixels);
        if let Some(dir) = &self.dir {
            if std::fs::create_dir_all(dir).is_ok() {
                let _ = std::fs::write(dir.join(format!("{}.png", file_stem(&host))), &png);
            }
        }
        self.views.insert(view, host.clone());
        self.insert(host, Some(Icon { image: to_image(pixels), png, scaled: None }));
    }

    pub fn get(&mut self, host: &str, size: usize) -> Option<&Image> {
        if !self.cache.contains_key(host) {
            let icon = self.dir.as_ref().and_then(|dir| {
                let png = std::fs::read(dir.join(format!("{}.png", file_stem(host)))).ok()?;
                let pixels = decode_png(&png)?;
                Some(Icon { image: to_image(&pixels), png, scaled: None })
            });
            self.insert(host.to_string(), icon);
        }
        let icon = self.cache.get_mut(host)?.as_mut()?;
        if size == STORE_PX {
            return Some(&icon.image);
        }
        if icon.scaled.as_ref().is_none_or(|(old, _)| *old != size) {
            icon.scaled = Some((size, icon.image.resized(size, size)));
        }
        icon.scaled.as_ref().map(|(_, image)| image)
    }

    /// Servo exposes decoded pixels rather than the selected source URL; return those actual pixels as a data URL.
    pub fn url_of(&mut self, view: WebViewId, page_url: &str) -> Option<String> {
        let host = self.views.get(&view).cloned().or_else(|| host_key(page_url))?;
        self.get(&host, STORE_PX)?;
        let png = &self.cache.get(&host)?.as_ref()?.png;
        Some(format!("data:image/png;base64,{}", bifrost_core::plist::base64_encode(png)))
    }

    pub fn forget(&mut self, view: WebViewId) {
        self.views.remove(&view);
    }
}

fn to_image(pixels: &[u8]) -> Image {
    let mut image = Image::new(STORE_PX, STORE_PX);
    for (out, pixel) in image.px.iter_mut().zip(pixels.chunks_exact(4)) {
        *out = rgba(pixel[0], pixel[1], pixel[2], pixel[3]);
    }
    image
}

fn encode_png(pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, STORE_PX as u32, STORE_PX as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    if let Ok(mut writer) = encoder.write_header() {
        let _ = writer.write_image_data(pixels);
    }
    bytes
}

fn decode_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    if reader.info().width != STORE_PX as u32 || reader.info().height != STORE_PX as u32 {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    pixels.truncate(frame.buffer_size());
    Some(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_pixels_survive_cache_and_png_round_trip() {
        let mut icons = Favicons::new(None);
        let pixels = vec![128; STORE_PX * STORE_PX * 4];
        icons.on_native(7, "https://example.com/path", &pixels);
        let data_url = icons.url_of(7, "https://example.com/path").unwrap();
        let encoded = data_url.strip_prefix("data:image/png;base64,").unwrap();
        let png = bifrost_core::plist::base64_decode(encoded).unwrap();
        assert_eq!(decode_png(&png), Some(pixels));
        assert_eq!(icons.get("example.com", 16).unwrap().w, 16);
    }

    #[test]
    fn origins_and_cache_paths_do_not_alias() {
        assert_eq!(host_key("https://user:pw@EXAMPLE.com:8443/x").as_deref(), Some("example.com:8443"));
        assert_ne!(host_key("https://www.example.com"), host_key("https://example.com"));
        assert_ne!(file_stem("example.com:8443"), file_stem("example.com_8443"));
        assert!(host_key("data:image/png,abc").is_none());
    }

    #[test]
    fn native_icon_working_set_is_bounded() {
        let mut icons = Favicons::new(None);
        for i in 0..=MAX_MEMORY_ICONS {
            icons.on_native(1, &format!("https://site-{i}.example/"), &vec![255; STORE_PX * STORE_PX * 4]);
        }
        assert_eq!(icons.cache.len(), MAX_MEMORY_ICONS);
        assert!(!icons.cache.contains_key("site-0.example"));
    }
}
