//! The video thread: access units in (every one, in order), the newest picture out. Decoding is
//! the hardware H.264 decoder. While the shell can draw video on the GPU (`gpu` set)
//! the picture stays in the decoder's buffer and goes out as `Picture::Gpu`; otherwise the NV12
//! output is converted and scaled to the letterboxed view size in one pass, so the shell only
//! copies the result onto the screen.

use crate::media::{convert, hwdec, Codec, Frame as Decoded, Picture, Pixels, VideoDecoder};
use crate::gfx::Image;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;
use wowstream_client::client::Client;
use wowstream_client::proto::Frame;

/// Threads for the NV12 conversion.
const CONVERT_THREADS: usize = 3;

/// Wakes the shell's poll loop.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The newest picture; the shell takes it.
pub type Latest = Arc<Mutex<Option<Picture>>>;

pub enum Msg {
    /// A complete Annex-B access unit.
    Frame(Frame),
    /// A new stream (connect or resize): drop the decoder, wait for an IDR.
    Restart { width: u16, height: u16 },
    /// Where the picture is shown, in pixels.
    View { width: usize, height: usize },
}

/// Starts the video thread; it ends (dropping its hardware decoder) once `stop` is set or the
/// channel closes. The handle lets the session wait for that before a new one opens a decoder.
/// `gpu`: the shell draws pictures on the GPU, so leave them in the decoder's buffers.
pub fn spawn(rx: Receiver<Msg>, client: Arc<Client>, latest: Latest, wake: Wake, stop: Arc<AtomicBool>, gpu: Arc<AtomicBool>) -> Option<std::thread::JoinHandle<()>> {
    let run = move || Video { client, latest, wake, decoder: None, zero_copy: false, gpu, stream: (1280, 720), view: (1920, 1200), waiting_idr: true }.run(rx, &stop);
    match std::thread::Builder::new().name("wow-video".into()).spawn(run) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("springboard: wow: video thread: {e}");
            None
        }
    }
}

struct Video {
    client: Arc<Client>,
    latest: Latest,
    wake: Wake,
    decoder: Option<Box<dyn VideoDecoder>>,
    /// the decoder hands out pictures in its own buffers
    zero_copy: bool,
    gpu: Arc<AtomicBool>,
    stream: (u16, u16),
    view: (usize, usize),
    /// A fresh or failed decoder starts at an IDR; frames before it would decode to garbage.
    waiting_idr: bool,
}

impl Video {
    fn run(mut self, rx: Receiver<Msg>, stop: &AtomicBool) {
        while !stop.load(Relaxed) {
            let msg = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            match msg {
                Msg::View { width, height } => self.view = (width, height),
                Msg::Restart { width, height } => {
                    self.stream = (width, height);
                    self.reset();
                }
                Msg::Frame(f) => {
                    if self.waiting_idr && !f.keyframe {
                        continue;
                    }
                    self.waiting_idr = false;
                    if let Err(e) = self.decode(&f) {
                        eprintln!("springboard: wow: video: {e}");
                        self.reset();
                    }
                }
            }
        }
    }

    /// Forgets the decoder and asks the server for an IDR.
    fn reset(&mut self) {
        self.decoder = None;
        self.waiting_idr = true;
        self.client.request_keyframe();
    }

    fn decode(&mut self, f: &Frame) -> Result<(), String> {
        if self.decoder.is_none() {
            let mut d = hwdec::open(Codec::H264).map_err(|e| format!("open: {e}"))?;
            // Annex-B in, so there is no avcC to hand over; the SPS/PPS ride in the IDR.
            d.configure(Codec::H264, &[], self.stream.0 as u32, self.stream.1 as u32).map_err(|e| format!("configure: {e}"))?;
            self.decoder = Some(d);
            self.zero_copy = false;
        }
        let decoder = self.decoder.as_mut().expect("opened above");
        let want = self.gpu.load(Relaxed);
        if want != self.zero_copy {
            self.zero_copy = decoder.set_zero_copy(want) && want;
        }
        let began = Instant::now();
        decoder.push(&f.data, f.pts_us as i64).map_err(|e| format!("push: {e}"))?;
        let mut newest = None;
        while let Some(frame) = decoder.pull().map_err(|e| format!("pull: {e}"))? {
            newest = Some(frame);
        }
        // Several pictures out of one push (a burst): only the newest is worth converting.
        let Some(frame) = newest else { return Ok(()) };
        let picture = match frame.pixels {
            Pixels::Dma(p) if self.zero_copy => Picture::Gpu(p),
            _ => Picture::Rgba(convert_frame(&frame, self.view)),
        };
        self.client.stats().add_decode_sample(began.elapsed().as_micros() as u32);
        // the picture it replaces (never shown) goes back to the decoder here
        *self.latest.lock().unwrap() = Some(picture);
        (self.wake)();
        Ok(())
    }
}

/// The picture fitted into `view` (aspect kept) as premultiplied RGBA.
fn convert_frame(frame: &Decoded, view: (usize, usize)) -> Image {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let (ow, oh) = convert::fit(frame.width, frame.height, view.0 as u32, view.1 as u32);
    match &frame.pixels {
        Pixels::Nv12 { y, uv, stride } => convert::nv12_to_rgba(convert::Nv12 { y, uv, stride: *stride as usize, w, h }, ow, oh, convert::matrix_for(frame.height), CONVERT_THREADS),
        Pixels::Rgba(img) => convert::rgba_scale(img, ow, oh),
        Pixels::Dma(p) => convert::dma_to_rgba(p, ow, oh, convert::matrix_for(frame.height), CONVERT_THREADS),
    }
}
