//! Free drawing helpers for desk windows: shadows, rims, traffic lights, corner clipping and glass kinds.

use super::*;

/// only the expensive per-frame SDF coverage walks are replaced by a clipped source-over blit.
pub(super) fn cached_window_shadow(s: &mut Surface, ui: &crate::shell::ui::Ui, x: f32, y: f32, w: f32, h: f32, rad: f32, focused: bool) {
    use std::cell::RefCell;
    type Key = (usize, usize, u32, u32, u32, bool);
    const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
    const MAX_CACHE_ENTRIES: usize = 8;
    thread_local! { static CACHE: RefCell<Vec<(Key, usize, usize, Image)>> = const { RefCell::new(Vec::new()) }; }
    let (layers, step_pt, dy_pt, alpha) = if focused { (8, 5.0, 12.0, 9) } else { (4, 4.0, 5.0, 8) };
    let step = ui.pt(step_pt);
    let dy = ui.pt(dy_pt);
    let margin = (step * layers as f32 + dy + 2.0).ceil() as usize;
    let (width, height) = (w.round() as usize, h.round() as usize);
    let key = (width, height, rad.to_bits(), step.to_bits(), dy.to_bits(), focused);
    let Some(image_w) = width.checked_add(margin.saturating_mul(2)) else {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    };
    let Some(image_h) = height.checked_add(margin.saturating_mul(2)) else {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    };
    let Some(image_bytes) = image_w.checked_mul(image_h).and_then(|pixels| pixels.checked_mul(std::mem::size_of::<u32>())) else {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    };
    if image_bytes > MAX_CACHE_BYTES {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = match cache.iter().position(|(candidate, _, _, _)| *candidate == key) {
            Some(index) => index,
            None => {
                while cache.len() >= MAX_CACHE_ENTRIES || cache.iter().map(|(_, _, bytes, _)| *bytes).sum::<usize>() + image_bytes > MAX_CACHE_BYTES {
                    if cache.is_empty() { break; }
                    cache.remove(0);
                }
                let mut image = Image::new(image_w, image_h);
                let mut shadow = Surface { w: image.w, h: image.h, stride: image.w, px: &mut image.px, y0: 0 };
                for k in (0..layers).rev() {
                    let g = step * (k + 1) as f32;
                    shadow.fill_rrect(margin as f32 - g, margin as f32 - g + dy, width as f32 + 2.0 * g, height as f32 + 2.0 * g, rad + g, rgba(0, 0, 0, alpha));
                }
                shadow.fill_rrect(margin as f32 - 1.0, margin as f32 - 1.0, width as f32 + 2.0, height as f32 + 2.0, rad + 1.0, rgba(0, 0, 0, 40));
                cache.push((key, margin, image_bytes, image));
                cache.len() - 1
            }
        };
        let (_, margin, _, image) = &cache[index];
        s.blit(image, x.round() as i32 - *margin as i32, y.round() as i32 - *margin as i32, 255, 255);
    });
}

pub(super) fn draw_window_shadow_layers(s: &mut Surface, ui: &crate::shell::ui::Ui, x: f32, y: f32, w: f32, h: f32, rad: f32, focused: bool) {
    let (layers, step_pt, dy_pt, alpha) = if focused { (8, 5.0, 12.0, 9) } else { (4, 4.0, 5.0, 8) };
    let step = ui.pt(step_pt);
    let dy = ui.pt(dy_pt);
    for k in (0..layers).rev() {
        let g = step * (k + 1) as f32;
        s.fill_rrect(x - g, y - g + dy, w + 2.0 * g, h + 2.0 * g, rad + g, rgba(0, 0, 0, alpha));
    }
    s.fill_rrect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, rad + 1.0, rgba(0, 0, 0, 40));
}

/// A traffic light as a glossy sphere: a rim in a darker shade of its colour and a body that is lighter towards the bottom.
pub(super) fn draw_light(s: &mut Surface, lr: &Rect, c: u32, focused: bool, px: f32) {
    let (x, y, d) = (lr.x as f32, lr.y as f32, lr.w as f32);
    let shade = |c: u32, to: u32, k: f32| bifrost_render::appearance::mix(c, to, k);
    let rim = shade(c, crate::gfx::pack(0, 0, 0, 255), if focused { 0.22 } else { 0.18 });
    s.fill_rrect(x, y, d, d, d / 2.0, rim);
    let r = d / 2.0 - px;
    let rows = (r * 2.0).ceil() as i32;
    let (top, bottom) = (shade(c, crate::gfx::pack(255, 255, 255, 255), 0.10), shade(c, crate::gfx::pack(255, 255, 255, 255), 0.38));
    for row in 0..rows {
        let dy = row as f32 + 0.5 - r;
        let half = (r * r - dy * dy).max(0.0).sqrt();
        if half <= 0.0 {
            continue;
        }
        let k = row as f32 / rows.max(1) as f32;
        s.fill_rrect(x + d / 2.0 - half, y + px + row as f32, half * 2.0, 1.0, 0.0, shade(top, bottom, k));
    }
}

/// The glyph a traffic light shows while the pointer is over the three: close x, minimise -, zoom +
/// (an inset square, hollow, when the window is already maximised).
pub(super) fn draw_light_glyph(s: &mut Surface, lr: &Rect, i: usize, maximized: bool, p: f32) {
    let (cx, cy) = (lr.x as f32 + lr.w as f32 / 2.0, lr.y as f32 + lr.h as f32 / 2.0);
    let a = lr.w as f32 * 0.22;
    let th = (p * 2.0).max(2.2);
    let dot = |s: &mut Surface, x: f32, y: f32| s.fill_rrect(x - th / 2.0, y - th / 2.0, th, th, th / 2.0, TRAFFIC_GLYPH);
    let steps = (a * 2.0 / th * 2.0).ceil().max(2.0) as i32;
    for k in 0..=steps {
        let u = -a + 2.0 * a * k as f32 / steps as f32;
        match i {
            0 => { dot(s, cx + u, cy + u); dot(s, cx + u, cy - u); }
            1 => dot(s, cx + u, cy),
            _ if maximized => { dot(s, cx + u, cy - a); dot(s, cx + u, cy + a); dot(s, cx - a, cy + u); dot(s, cx + a, cy + u); }
            _ => { dot(s, cx + u, cy); dot(s, cx, cy + u); }
        }
    }
}

/// The green light's glyph on a full-screen window (the band revealed under the menu bar): two solid arrowheads, at
/// the top-left and bottom-right of the centre, whose right-angle corners point in at it, as the reference's "Exit Full
/// Screen" glyph does (the opposite of the outward pair that enters full screen).
#[cfg(any(feature = "desktop-wayland", test))]
pub(super) fn draw_exit_full_screen_glyph(s: &mut Surface, lr: &Rect, p: f32) {
    let (cx, cy) = (lr.x as f32 + lr.w as f32 / 2.0, lr.y as f32 + lr.h as f32 / 2.0);
    let gap = (p * 0.6).max(0.6);
    let len = lr.w as f32 * 0.28;
    let rows = len.ceil() as i32;
    for r in 0..rows {
        let d = r as f32;
        let run = (len - d).max(0.0);
        if run <= 0.0 {
            break;
        }
        // top-left: the corner at (cx - gap, cy - gap), legs to the left and upwards
        s.fill_rrect(cx - gap - run, cy - gap - d - 1.0, run, 1.0, 0.0, TRAFFIC_GLYPH);
        // bottom-right: the corner at (cx + gap, cy + gap), legs to the right and downwards
        s.fill_rrect(cx + gap, cy + gap + d, run, 1.0, 0.0, TRAFFIC_GLYPH);
    }
}

pub(super) fn copy_rect(s: &mut Surface, src: &[u32], w: usize, h: usize, x: i32, y: i32) {
    let (x0, x1) = (x.max(0), (x + w as i32).min(s.w as i32));
    if x0 >= x1 {
        return;
    }
    let n = (x1 - x0) as usize;
    for row in 0..h as i32 {
        let sy = y + row;
        if sy < s.y0 as i32 || sy >= s.h as i32 {
            continue;
        }
        let from = row as usize * w + (x0 - x) as usize;
        let to = (sy as usize - s.y0) * s.stride + x0 as usize;
        s.px[to..to + n].copy_from_slice(&src[from..from + n]);
    }
}

/// `copy_rect` with the four corners of the rectangle blended over what is already there through a rounded-rect mask.
pub(super) fn copy_rect_rounded(s: &mut Surface, src: &[u32], w: usize, h: usize, x: i32, y: i32, radius: f32) {
    let n = (radius.ceil() as usize).min(w / 2).min(h / 2);
    let mask = if n > 0 { crate::gfx::rrect_mask(n * 2, n * 2, radius) } else { Vec::new() };
    // the destination under each corner pixel, read before the body overwrites it
    let mut under: Vec<(usize, u32, u32)> = Vec::new();
    for cy in 0..n {
        for cx in 0..n {
            let cov = mask[cy * 2 * n + cx] as u32;
            if cov >= 255 {
                continue;
            }
            for (px, py) in [(cx, cy), (w - 1 - cx, cy), (cx, h - 1 - cy), (w - 1 - cx, h - 1 - cy)] {
                let (dx, dy) = (x + px as i32, y + py as i32);
                if dx < 0 || dx >= s.w as i32 || dy < s.y0 as i32 || dy >= s.h as i32 {
                    continue;
                }
                let at = (dy as usize - s.y0) * s.stride + dx as usize;
                under.push((at, s.px[at], cov));
                let _ = (px, py);
            }
        }
    }
    copy_rect(s, src, w, h, x, y);
    for (at, was, cov) in under {
        let now = s.px[at];
        let ch = |i: u32| ((((now >> i) & 255) * cov + ((was >> i) & 255) * (255 - cov)) / 255) << i;
        s.px[at] = ch(0) | ch(8) | ch(16) | ch(24);
    }
}

/// Dark appearance: a white ~15% hairline just inside the window edge (Golden Gate's window rim);
/// in Light it fades to nothing and the dark outer hairline (shadow pass) edges the window.
pub(super) fn window_rim(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, rad: f32) {
    stroke_window_rim(s, x, y, w, h, rad, window_rim_color());
}

pub(super) fn window_rim_color() -> u32 {
    bifrost_render::appearance::Pair::new(rgba(255, 255, 255, 38), rgba(255, 255, 255, 0)).now()
}

pub(super) fn stroke_window_rim(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, rad: f32, c: u32) {
    if c >> 24 != 0 { s.stroke_rrect(x, y, w, h, rad, 1.0, c); }
}

/// Width of the band along each window edge that the rim stroke and the corner clip can change.
#[cfg(any(test, feature = "desktop-wayland"))]
pub(super) const EDGE_BAND: usize = 3;

/// The parts of a `width` x `height` window that the rim stroke and the corner clip touch, as (x, y, w, h): a band
/// `EDGE_BAND` wide along each edge and a square over each rounded corner.
#[cfg(any(feature = "desktop-wayland", test))]
pub(super) fn edge_band_rects(width: usize, height: usize, radius: f32) -> [(i32, i32, i32, i32); 8] {
    let n = (radius.ceil() as usize).min(width / 2).min(height / 2);
    let (w, h) = (width as i32, height as i32);
    let b = EDGE_BAND.min(width).min(height) as i32;
    let c = (n + EDGE_BAND).min(width).min(height) as i32;
    [(0, 0, w, b), (0, h - b, w, b), (0, 0, b, h), (w - b, 0, b, h),
     (0, 0, c, c), (w - c, 0, c, c), (0, h - c, c, c), (w - c, h - c, c, c)]
}

/// Whether window rect `r` overlaps body damage area `a` (body coordinates, `top` rows below the window's top).
#[cfg(feature = "desktop-wayland")]
pub(super) fn overlaps(r: &(i32, i32, i32, i32), a: &crate::gfx::Area, top: usize) -> bool {
    (a.x0 as i32) < r.0 + r.2 && (a.x1 as i32) > r.0 && ((a.y0 + top) as i32) < r.1 + r.3 && ((a.y1 + top) as i32) > r.1
}

/// Copies [x0, x1) x [y0, y1) of a retained window (window coordinates) from the app's frame `src` (pixels, width,
/// height; the frame starts `top` rows down), dimmed by `dim` for a background window as the fast path dims a fresh
/// blit. Pixels outside the frame (the title bar) are left as they are.
#[cfg(any(feature = "desktop-wayland", test))]
#[allow(clippy::too_many_arguments)]
pub(super) fn restore_from_frame(pixels: &mut [u32], width: usize, height: usize, top: usize, src: (&[u32], usize, usize), dim: Option<u32>, x0: i32, y0: i32, x1: i32, y1: i32) {
    let (spx, sw, sh) = src;
    let cx = |v: i32| (v.max(0) as usize).min(width).min(sw);
    let (x0, x1) = (cx(x0), cx(x1));
    let y0 = (y0.max(0) as usize).max(top);
    let y1 = (y1.max(0) as usize).min(height).min(top + sh);
    if x0 >= x1 { return; }
    for y in y0..y1 {
        let sy = y - top;
        let (Some(d), Some(s)) = (pixels.get_mut(y * width + x0..y * width + x1), spx.get(sy * sw + x0..sy * sw + x1)) else { continue };
        d.copy_from_slice(s);
        if let Some(amount) = dim { dim_rect(pixels, width, x0, y, x1, y + 1, amount); }
    }
}

/// Puts the inner rim and the rounded corners back on a retained external window whose app drew into the edge band.
/// Every band pixel inside the app's frame is first taken again from the frame (dimmed by `dim`), and every band pixel
/// outside it (the title bar, which already carries the rim from the full draw) is kept as it was; then the rim is
/// stroked once and the corners clipped once. So an edge pixel always holds exactly one stroke over the app's pixel,
/// however often the app repaints (stroking the retained buffer again blended 15% white per repaint, E2E-10).
/// Returns the window rects it changed (for the frame's damage).
#[cfg(any(feature = "desktop-wayland", test))]
#[allow(clippy::too_many_arguments)]
pub(super) fn restore_window_edge(pixels: &mut [u32], width: usize, height: usize, top: usize, src: (&[u32], usize, usize), dim: Option<u32>, radius: f32, rim: u32) -> Vec<(i32, i32, i32, i32)> {
    let rects = edge_band_rects(width, height, radius);
    let (_, sw, sh) = src;
    let in_frame = |x: usize, y: usize| y >= top && y - top < sh && x < sw;
    // pixels the frame does not cover keep their value: saved here, put back after the stroke and the clip
    let mut kept: Vec<(usize, u32)> = Vec::new();
    for &(x, y, w, h) in &rects {
        for py in y.max(0) as usize..((y + h).max(0) as usize).min(height) {
            for px in x.max(0) as usize..((x + w).max(0) as usize).min(width) {
                if !in_frame(px, py) { kept.push((py * width + px, pixels[py * width + px])); }
            }
        }
    }
    for &(x, y, w, h) in &rects { restore_from_frame(pixels, width, height, top, src, dim, x, y, x + w, y + h); }
    stroke_window_rim(&mut Surface { w: width, h: height, stride: width, px: &mut pixels[..], y0: 0 }, 0., 0., width as f32, height as f32, radius, rim);
    clip_window_corners(pixels, width, height, radius, None);
    for (i, v) in kept { pixels[i] = v; }
    rects.to_vec()
}

#[cfg(feature = "desktop-wayland")]
pub(super) fn clip_corners(image:&mut Image,radius:f32) {clip_window_corners(&mut image.px,image.w,image.h,radius,None);}
#[cfg(any(feature = "desktop-wayland", test))]
pub(super) fn clip_window_corners(pixels:&mut [u32],width:usize,height:usize,radius:f32,damage:Option<&crate::gfx::Damage>) {
    let n=(radius.ceil() as usize).min(width/2).min(height/2);
    let mask=crate::gfx::rrect_mask(n*2,n*2,radius);
    for y in 0..n {for x in 0..n {
        for (dx,dy,mx,my) in [(x,y,x,y),(width-1-x,y,2*n-1-x,y),(x,height-1-y,x,2*n-1-y),(width-1-x,height-1-y,2*n-1-x,2*n-1-y)] {
            if damage.is_some_and(|d|!d.areas().iter().any(|a|dx>=a.x0&&dx<a.x1&&dy>=a.y0&&dy<a.y1)) {continue;}
            let coverage=mask[my*2*n+mx] as u32; let p=&mut pixels[dy*width+dx];
            let channels=std::array::from_fn::<_,4,_>(|i|((*p>>(i*8))&255)*coverage/255);
            *p=channels.iter().enumerate().fold(0,|v,(i,c)|v|(c<<(i*8)));
        }
    }}
}

#[cfg(feature = "desktop-wayland")]
pub(super) fn linux_icon(app_id:&str) -> Image {
    let mut names=vec![app_id.to_owned(),app_id.to_ascii_lowercase()];
    let dirs=["/usr/share/applications", "/usr/local/share/applications"];
    for dir in dirs {
        if let Ok(entries)=std::fs::read_dir(dir) { for entry in entries.flatten() {
            if entry.path().extension().and_then(|s|s.to_str())!=Some("desktop") {continue;}
            if let Ok(text)=std::fs::read_to_string(entry.path()) {
                let stem=entry.path().file_stem().and_then(|s|s.to_str()).unwrap_or("").to_owned();
                if stem.eq_ignore_ascii_case(app_id) || text.lines().any(|l|l.strip_prefix("StartupWMClass=").is_some_and(|v|v.eq_ignore_ascii_case(app_id))) {
                    if let Some(icon)=text.lines().find_map(|l|l.strip_prefix("Icon=")) {names.insert(0,icon.to_owned());}
                }
            }
        }}
    }
    let mut paths=Vec::new();
    if app_id.to_ascii_lowercase().contains("firefox") {paths.push("/usr/lib/firefox/browser/chrome/icons/default/default128.png".to_owned());}
    for name in names {
        if name.starts_with('/') {paths.push(name.clone());}
        // Only desktop-entry basenames are accepted for theme lookup.
        if name.contains('/') {continue;}
        for size in [128,256,64,48,32] {paths.push(format!("/usr/share/icons/hicolor/{size}x{size}/apps/{name}.png"));}
        paths.push(format!("/usr/share/pixmaps/{name}.png"));
    }
    for path in paths {if let Ok(bytes)=std::fs::read(path) {if let Some(img)=crate::cg::imageio::decode(&bytes) {return img;}}}
    let mut img=Image::new(64,64);
    let mut surface=Surface {w:64,h:64,stride:64,px:&mut img.px,y0:0};
    surface.fill_rrect(0.0,0.0,64.0,64.0,12.0,crate::shell::ui::CELL);
    surface.fill_rrect(12.0,16.0,40.0,32.0,4.0,crate::shell::ui::SYSTEM_BLUE);
    img
}

/// Dock corner radius as a share of the dock's thickness (Golden Gate: ~24 pt on a 72 pt dock).
pub(super) const DOCK_RADIUS_K: f32 = 0.33;

/// A lighter glass edge: a bright 1 px inner rim over the tint (white ~35% in Light, ~18% in Dark).
/// Straight edges only; the rounded corners keep the outer hairline.
pub(super) fn dock_rim(s: &mut Surface, bar: Rect, rad: f32) {
    let c = bifrost_render::appearance::Pair::new(rgba(255, 255, 255, 46), rgba(255, 255, 255, 90)).now();
    let (x, y, w, h) = (bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32);
    let r = rad.ceil();
    s.fill_rrect(x + r, y + 1.0, (w - 2.0 * r).max(0.0), 1.0, 0.0, c);
    s.fill_rrect(x + r, y + h - 2.0, (w - 2.0 * r).max(0.0), 1.0, 0.0, c);
    s.fill_rrect(x + 1.0, y + r, 1.0, (h - 2.0 * r).max(0.0), 0.0, c);
    s.fill_rrect(x + w - 2.0, y + r, 1.0, (h - 2.0 * r).max(0.0), 0.0, c);
}

/// The dock's Trash, drawn procedurally: a light grey translucent bin, slightly tapered, with vertical ribs and a darker rim.
pub(super) fn draw_trash(s: &mut Surface, r: &Rect) {
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
    let (body, rib, rim) = (rgba(222, 226, 232, 200), rgba(160, 166, 176, 200), rgba(130, 136, 146, 230));
    let (top, bottom) = (y + h * 0.08, y + h * 0.98);
    let (tw, bw) = (w * 0.80, w * 0.66);
    let rows = (bottom - top).max(1.0) as i32;
    for k in 0..rows {
        let f = k as f32 / rows as f32;
        let row = tw + (bw - tw) * f;
        let (x0, yk) = (x + (w - row) / 2.0, top + k as f32);
        s.fill_rrect(x0, yk, row, 1.0, 0.0, body);
        for j in 1..5 { s.fill_rrect(x0 + row * j as f32 / 5.0 - 0.5, yk, 1.0, 1.0, 0.0, rib); }
        s.fill_rrect(x0, yk, 1.0, 1.0, 0.0, rim);
        s.fill_rrect(x0 + row - 1.0, yk, 1.0, 1.0, 0.0, rim);
    }
    s.fill_rrect(x + (w - bw) / 2.0, bottom - 1.0, bw, 1.5, 0.75, rim);
    let lip = (h * 0.06).max(2.0);
    s.fill_rrect(x + (w - tw * 1.06) / 2.0, top - lip / 2.0, tw * 1.06, lip, lip / 2.0, rim);
}

/// A soft drop shadow made of concentric rings just outside a rounded rectangle, so the glass tint
/// inside it is not darkened. It reaches `SHADOW_REACH_PT` points beyond the shape. The rings are
/// drawn once per size into a cached image (stroking them every frame cost 8 ms for the dock).
pub(in crate::shell) fn soft_shadow(s: &mut Surface, ui: &crate::shell::ui::Ui, x: f32, y: f32, w: f32, h: f32, rad: f32) {
    use std::cell::RefCell;
    type Key = (i32, i32, u32, u32);
    thread_local! { static CACHE: RefCell<Vec<(Key, Image)>> = const { RefCell::new(Vec::new()) }; }
    let step = ui.pt(SHADOW_REACH_PT / 4.0);
    let margin = (step * 4.0).ceil() as usize + ui.pt(2.0).ceil() as usize;
    let key: Key = (w.round() as i32, h.round() as i32, rad.to_bits(), step.to_bits());
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let i = match c.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let mut img = Image::new(w.round() as usize + 2 * margin, h.round() as usize + 2 * margin);
                let mut t = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
                for (k, a) in [30u8, 21, 13, 7].into_iter().enumerate() {
                    let g = step * (k + 1) as f32;
                    t.stroke_rrect(margin as f32 - g, margin as f32 - g + ui.pt(2.0), w.round() + 2.0 * g, h.round() + 2.0 * g, rad + g, step, rgba(0, 0, 0, a));
                }
                if c.len() >= 8 { c.remove(0); }
                c.push((key, img));
                c.len() - 1
            }
        };
        s.blit(&c[i].1, x.round() as i32 - margin as i32, y.round() as i32 - margin as i32, 255, 255);
    });
}

/// How far the dock and menu shadows reach beyond their shape, in points.
pub(in crate::shell) const SHADOW_REACH_PT: f32 = 10.0;

/// How much the body of a window that is not in front is darkened (of 255): inactive windows recede.
pub(in crate::shell) const BACKGROUND_DIM: u32 = 36;

/// Darkens the opaque pixels in [x0, x1) x [y0, y1) of `px` (row length `stride`) by `amount` of 255.
pub(in crate::shell) fn dim_rect(px: &mut [u32], stride: usize, x0: usize, y0: usize, x1: usize, y1: usize, amount: u32) {
    let keep = 255 - amount.min(255);
    for y in y0..y1 {
        let Some(row) = px.get_mut(y * stride + x0..y * stride + x1.min(stride)) else { continue };
        for p in row {
            let ch = |sh: u32| (((*p >> sh) & 0xff) * keep / 255) << sh;
            *p = (*p & 0xff00_0000) | ch(16) | ch(8) | ch(0);
        }
    }
}

/// One material region (window px) as the blur region of its window: unless maximized, each side that lies inside the
/// window grows by the window radius (the extra blur sits under opaque pixels) and the corner takes the window radius.
#[cfg(feature = "desktop-wayland")]
pub(super) fn expand_material((x, y, rw, rh, r, kind, active): (f32, f32, f32, f32, f32, bifrost_render::glass::Kind, Option<bool>), (ww, wh): (f32, f32), maximized: bool, corner: f32) -> bifrost_render::glass::Rect {
    if maximized {
        return bifrost_render::glass::Rect { x, y, w: rw, h: rh, r, kind, active, detail: None, tint: [0.0; 4] };
    }
    let e = 0.5;
    let x0 = if x <= e { 0.0 } else { (x - corner).max(0.0) };
    let y0 = if y <= e { 0.0 } else { (y - corner).max(0.0) };
    let x1 = if x + rw >= ww - e { ww } else { (x + rw + corner).min(ww) };
    let y1 = if y + rh >= wh - e { wh } else { (y + rh + corner).min(wh) };
    bifrost_render::glass::Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0, r: corner.max(r), kind, active, detail: None, tint: [0.0; 4] }
}

/// The presenter's glass kind for a window material an app declared (`NativeVisualEffectView.Material`).
#[cfg(feature = "desktop-wayland")]
pub(super) fn glass_kind(k: bifrost_proto::MaterialKind) -> bifrost_render::glass::Kind {
    use bifrost_proto::MaterialKind as M;
    use bifrost_render::glass::Kind;
    match k {
        M::Sidebar => Kind::Sidebar,
        M::Titlebar | M::HeaderView => Kind::Titlebar,
        M::WindowBackground | M::ContentBackground | M::UnderWindowBackground | M::UnderPageBackground => Kind::Content,
        M::Menu | M::Popover | M::Sheet | M::HudWindow | M::FullScreenUi | M::ToolTip | M::Selection => Kind::Menu,
    }
}

/// `NativeVisualEffectView.State` for the presenter: `None` follows the window's active state.
#[cfg(feature = "desktop-wayland")]
pub(super) fn glass_active(s: bifrost_proto::MaterialState) -> Option<bool> {
    match s {
        bifrost_proto::MaterialState::FollowsWindowActiveState => None,
        bifrost_proto::MaterialState::Active => Some(true),
        bifrost_proto::MaterialState::Inactive => Some(false),
    }
}
