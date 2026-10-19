//! Resize and present shared-memory web content.
use super::*;

impl Engine {

    /// Applies the last `size` of a burst, so a live window resize resizes Servo once per loop turn.
    pub(super) fn apply_size(&mut self) {
        let Some((size, scale)) = self.pending_size.take() else { return };
        self.size = size;
        // WebView::resize resizes the shared RenderingContext and relayouts every view on it (servo
        // paint/painter.rs resize_rendering_context). It returns early when the context already has the
        // size, so the context is never resized directly while a view exists.
        match self.views.values().next() {
            Some(v) => v.resize(size),
            None => self.rc.resize(size),
        }
        for v in self.views.values() {
            if scale != self.scale {
                v.set_hidpi_scale_factor(Scale::new(scale));
            }
        }
        self.scale = scale;
        self.dirty = true;
    }

    /// Paints the visible view and hands the pixels to the app, at most one frame in flight.
    pub(super) fn present(&mut self) {
        if !self.dirty || self.awaiting_ack || self.suspended {
            return;
        }
        let Some(v) = self.visible_view() else { return };
        v.paint();
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let rect = euclid::default::Box2D::new(euclid::point2(0, 0), euclid::point2(w as i32, h as i32)).cast_unit();
        if w == 0 || h == 0 {
            self.dirty = false;
            return;
        }
        let Some(img) = self.rc.read_to_image(rect) else { return };
        if img.width() as usize != w || img.height() as usize != h {
            self.dirty = false;
            return;
        }
        if w * h * 4 > MAX_DIM as usize * MAX_DIM as usize * 4 || !self.shm.ensure(w * h * 4) {
            // keep dirty: retry once the app has grown the frame file
            if !self.shm_warned {
                self.shm_warned = true;
                say(&format!("crashed 4294967295 frame file too small for {w}x{h}"));
            }
            return;
        }
        self.shm_warned = false;
        self.dirty = false;
        self.shm.write_rgba(img.as_raw(), w * h);
        self.awaiting_ack = true;
        // the view id lets the app drop a frame that was in flight when the tab changed
        say(&format!("frame {w} {h} {}", self.visible.unwrap_or(u32::MAX)));
    }
}
