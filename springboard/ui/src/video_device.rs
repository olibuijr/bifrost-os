//! Video devices and stream formats: the capture nodes the system layer finds, the pixel formats each one
//! streams in, and the rules that pick one device and one format for a stream. This is pure. The system
//! layer probes the capture nodes and hands each one over as a `VideoDevice`; this module decides which
//! node is usable, which format to negotiate, how large one frame is and how many buffers a stream holds.
//! Nothing here opens a device or touches memory that a device writes to.

/// Capability bit: the node captures single-planar video.
pub const CAP_VIDEO_CAPTURE: u32 = 0x1;
/// Capability bit: the node captures multi-planar video. Such a node is not usable here.
pub const CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x1000;
/// Capability bit: the node is a metadata capture node. Such a node is not usable here.
pub const CAP_META_CAPTURE: u32 = 0x0080_0000;
/// Capability bit: the node can stream buffers.
pub const CAP_STREAMING: u32 = 0x0400_0000;
/// Capability bit: the per-node capability word is valid and must be used instead of the device word.
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;

/// How many buffers a stream keeps in flight. A BifrOSt bound, not a value from the reference.
pub const BUFFER_COUNT: u32 = 4;

/// Whether a capture node can stream single-planar video. A node that reports the device word is judged
/// on that word, and a metadata node is never usable.
pub fn capture_node_usable(capabilities: u32, device_caps: u32) -> bool {
    let caps = if capabilities & CAP_DEVICE_CAPS != 0 { device_caps } else { capabilities };
    caps & CAP_VIDEO_CAPTURE != 0 && caps & CAP_META_CAPTURE == 0 && caps & CAP_STREAMING != 0
}

/// A pixel format a stream can deliver. Only the four formats the converter reads are named; any other
/// four-character code is kept as `Other` and is never chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Packed YUV 4:2:2, two pixels in four bytes.
    Yuyv,
    /// Planar YUV 4:2:0 with interleaved UV.
    Nv12,
    /// Planar YUV 4:2:0 with interleaved VU.
    Nv21,
    /// Motion JPEG: each frame is a compressed image of driver-chosen size.
    Mjpeg,
    /// Any other format, by its four-character code.
    Other(u32),
}

/// Packs four ASCII bytes into a code, first byte lowest, as the kernel does.
pub const fn fourcc(s: &[u8; 4]) -> u32 {
    s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24
}

impl PixelFormat {
    /// The format for a four-character code.
    pub fn from_fourcc(code: u32) -> PixelFormat {
        match code {
            c if c == fourcc(b"YUYV") => PixelFormat::Yuyv,
            c if c == fourcc(b"NV12") => PixelFormat::Nv12,
            c if c == fourcc(b"NV21") => PixelFormat::Nv21,
            c if c == fourcc(b"MJPG") => PixelFormat::Mjpeg,
            other => PixelFormat::Other(other),
        }
    }

    /// The four-character code of this format.
    pub fn fourcc(self) -> u32 {
        match self {
            PixelFormat::Yuyv => fourcc(b"YUYV"),
            PixelFormat::Nv12 => fourcc(b"NV12"),
            PixelFormat::Nv21 => fourcc(b"NV21"),
            PixelFormat::Mjpeg => fourcc(b"MJPG"),
            PixelFormat::Other(code) => code,
        }
    }

    /// The order in which formats are preferred when a device offers several: the cheapest raw format
    /// first, since converting raw YUV costs the least CPU. `None` for formats that cannot be converted.
    pub fn preference(self) -> Option<usize> {
        match self {
            PixelFormat::Yuyv => Some(0),
            PixelFormat::Nv12 => Some(1),
            PixelFormat::Nv21 => Some(2),
            PixelFormat::Mjpeg => Some(3),
            PixelFormat::Other(_) => None,
        }
    }

    /// Bytes in one row of a frame of this width. `None` when the width is zero, the format is compressed,
    /// or the width breaks the format's pixel grouping (YUYV needs an even width).
    pub fn bytes_per_line(self, width: u32) -> Option<usize> {
        if width == 0 {
            return None;
        }
        match self {
            PixelFormat::Yuyv if width % 2 == 0 => Some(width as usize * 2),
            PixelFormat::Nv12 | PixelFormat::Nv21 => Some(width as usize),
            _ => None,
        }
    }

    /// Bytes in one uncompressed frame. `None` for compressed formats, zero sizes, and sizes that break
    /// the format's grouping: YUYV needs an even width, and the 4:2:0 formats need even width and height.
    pub fn frame_bytes(self, width: u32, height: u32) -> Option<usize> {
        if height == 0 {
            return None;
        }
        let line = self.bytes_per_line(width)?;
        match self {
            PixelFormat::Yuyv => Some(line * height as usize),
            PixelFormat::Nv12 | PixelFormat::Nv21 if height % 2 == 0 => Some(line * height as usize * 3 / 2),
            _ => None,
        }
    }
}

/// One size and format a device can stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
}

impl StreamFormat {
    /// True when the format can be streamed at this size: it is convertible, and for uncompressed formats
    /// the size respects the format's grouping.
    pub fn is_valid(&self) -> bool {
        self.format.preference().is_some()
            && self.width > 0
            && self.height > 0
            && (self.format == PixelFormat::Mjpeg || self.format.frame_bytes(self.width, self.height).is_some())
    }
}

/// Picks the format a stream negotiates. Formats are ranked first by `PixelFormat::preference`, then by
/// the distance from the wanted size (sum of the width and height differences), then by the larger frame.
/// Invalid formats are skipped. Returns `None` when no valid format is offered.
pub fn choose_format(formats: &[StreamFormat], want_width: u32, want_height: u32) -> Option<StreamFormat> {
    formats
        .iter()
        .filter(|f| f.is_valid())
        .min_by_key(|f| {
            let rank = f.format.preference().unwrap_or(usize::MAX);
            let distance = f.width.abs_diff(want_width) as u64 + f.height.abs_diff(want_height) as u64;
            (rank, distance, u64::MAX - f.width as u64 * f.height as u64)
        })
        .copied()
}

/// The index N of a `/dev/videoN` path, or `None` for any other path.
pub fn device_index(path: &str) -> Option<u32> {
    let digits = path.strip_prefix("/dev/video")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// A capture node as the system layer found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoDevice {
    /// The device node, such as `/dev/video0`.
    pub path: String,
    /// The card name the driver reports.
    pub card: String,
    /// The node's capability word.
    pub capabilities: u32,
    /// The device capability word, used when `CAP_DEVICE_CAPS` is set.
    pub device_caps: u32,
    /// Every size and format the node reports.
    pub formats: Vec<StreamFormat>,
}

impl VideoDevice {
    /// True when the node can stream and offers at least one valid format.
    pub fn is_usable(&self) -> bool {
        capture_node_usable(self.capabilities, self.device_caps) && self.formats.iter().any(|f| f.is_valid())
    }
}

/// The lowest-numbered usable device. Devices without a `/dev/videoN` path sort last.
pub fn select_device(devices: &[VideoDevice]) -> Option<&VideoDevice> {
    devices
        .iter()
        .filter(|d| d.is_usable())
        .min_by_key(|d| device_index(&d.path).unwrap_or(u32::MAX))
}

/// Why a stream cannot be planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// No device in the list is usable.
    NoUsableDevice,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::NoUsableDevice => write!(f, "no usable video device"),
        }
    }
}

/// The settings a stream starts with: which node, which format, and the buffers it cycles through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamPlan {
    pub path: String,
    pub format: StreamFormat,
    /// Buffers in flight, always `BUFFER_COUNT`.
    pub buffers: u32,
    /// Bytes per buffer. `None` for compressed formats, whose frame size the driver sets per frame.
    pub buffer_bytes: Option<usize>,
}

/// Plans a stream from the lowest-numbered usable device at the nearest format to the wanted size.
pub fn plan_stream(devices: &[VideoDevice], want_width: u32, want_height: u32) -> Result<StreamPlan, StreamError> {
    let device = select_device(devices).ok_or(StreamError::NoUsableDevice)?;
    let format = choose_format(&device.formats, want_width, want_height).ok_or(StreamError::NoUsableDevice)?;
    Ok(StreamPlan {
        path: device.path.clone(),
        format,
        buffers: BUFFER_COUNT,
        buffer_bytes: format.format.frame_bytes(format.width, format.height),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAMING_CAPTURE: u32 = CAP_VIDEO_CAPTURE | CAP_STREAMING;

    fn fmt(format: PixelFormat, width: u32, height: u32) -> StreamFormat {
        StreamFormat { format, width, height }
    }

    fn node(path: &str, capabilities: u32, formats: Vec<StreamFormat>) -> VideoDevice {
        VideoDevice { path: path.to_string(), card: "card".to_string(), capabilities, device_caps: 0, formats }
    }

    #[test]
    fn capture_node_usable_follows_the_capability_bits() {
        assert!(capture_node_usable(STREAMING_CAPTURE, 0));
        // A metadata node is never usable, even with capture and streaming set.
        assert!(!capture_node_usable(STREAMING_CAPTURE | CAP_META_CAPTURE, 0));
        // Without the streaming bit the node cannot deliver frames.
        assert!(!capture_node_usable(CAP_VIDEO_CAPTURE, 0));
        // A multi-planar-only node has no single-planar capture bit.
        assert!(!capture_node_usable(CAP_VIDEO_CAPTURE_MPLANE | CAP_STREAMING, 0));
        // With CAP_DEVICE_CAPS set, the device word decides and the node word is ignored.
        assert!(capture_node_usable(CAP_DEVICE_CAPS | CAP_VIDEO_CAPTURE_MPLANE, STREAMING_CAPTURE));
        assert!(!capture_node_usable(CAP_DEVICE_CAPS | STREAMING_CAPTURE, CAP_VIDEO_CAPTURE_MPLANE));
    }

    #[test]
    fn fourcc_codes_are_little_endian_and_round_trip() {
        assert_eq!(fourcc(b"YUYV"), 0x5659_5559);
        assert_eq!(fourcc(b"MJPG"), 0x4750_4A4D);
        assert_eq!(PixelFormat::from_fourcc(fourcc(b"NV21")), PixelFormat::Nv21);
        assert_eq!(PixelFormat::Mjpeg.fourcc(), fourcc(b"MJPG"));
        assert_eq!(PixelFormat::from_fourcc(fourcc(b"RGB3")), PixelFormat::Other(fourcc(b"RGB3")));
        assert_eq!(PixelFormat::Other(fourcc(b"RGB3")).preference(), None);
    }

    #[test]
    fn frame_bytes_follow_each_format_grouping() {
        // YUYV: 2 bytes per pixel.
        assert_eq!(PixelFormat::Yuyv.frame_bytes(640, 480), Some(614_400));
        // NV12 and NV21: 1.5 bytes per pixel.
        assert_eq!(PixelFormat::Nv12.frame_bytes(640, 480), Some(460_800));
        assert_eq!(PixelFormat::Nv21.bytes_per_line(640), Some(640));
        // Odd widths break the YUYV pair, odd heights break the 4:2:0 pair.
        assert_eq!(PixelFormat::Yuyv.frame_bytes(641, 480), None);
        assert_eq!(PixelFormat::Nv12.frame_bytes(640, 481), None);
        // Compressed frames and zero sizes have no fixed frame size.
        assert_eq!(PixelFormat::Mjpeg.frame_bytes(1280, 720), None);
        assert_eq!(PixelFormat::Yuyv.frame_bytes(0, 480), None);
    }

    #[test]
    fn choose_format_ranks_raw_formats_before_size() {
        // The raw YUYV format at 640x480 beats the exact-size MJPEG and NV12 offers, because the rank comes first.
        let offered = [fmt(PixelFormat::Mjpeg, 1280, 720), fmt(PixelFormat::Nv12, 1280, 720), fmt(PixelFormat::Yuyv, 640, 480)];
        assert_eq!(choose_format(&offered, 1280, 720), Some(fmt(PixelFormat::Yuyv, 640, 480)));
        // Within one format the size closest to the wanted one wins: 1000x700 is 580 away from 640x480 and 1300 from 1920x1080.
        let sizes = [fmt(PixelFormat::Yuyv, 1920, 1080), fmt(PixelFormat::Yuyv, 640, 480)];
        assert_eq!(choose_format(&sizes, 1000, 700), Some(fmt(PixelFormat::Yuyv, 640, 480)));
        // An invalid size (odd width for YUYV) is never chosen.
        let bad = [fmt(PixelFormat::Yuyv, 641, 480), fmt(PixelFormat::Mjpeg, 800, 600)];
        assert_eq!(choose_format(&bad, 640, 480), Some(fmt(PixelFormat::Mjpeg, 800, 600)));
        assert_eq!(choose_format(&[], 640, 480), None);
    }

    #[test]
    fn device_index_reads_only_video_nodes() {
        assert_eq!(device_index("/dev/video0"), Some(0));
        assert_eq!(device_index("/dev/video12"), Some(12));
        assert_eq!(device_index("/dev/video"), None);
        assert_eq!(device_index("/dev/videoX"), None);
        assert_eq!(device_index("/dev/media0"), None);
    }

    #[test]
    fn select_device_takes_the_lowest_usable_node() {
        let yuyv = [fmt(PixelFormat::Yuyv, 640, 480)];
        let devices = [
            node("/dev/video2", STREAMING_CAPTURE, vec![fmt(PixelFormat::Nv12, 640, 480)]),
            node("/dev/video0", CAP_META_CAPTURE | STREAMING_CAPTURE, yuyv.to_vec()),
            node("/dev/video1", STREAMING_CAPTURE, yuyv.to_vec()),
        ];
        // video0 is metadata and skipped; video1 is the lowest usable node.
        assert_eq!(select_device(&devices).map(|d| d.path.as_str()), Some("/dev/video1"));
        // A node with no valid format is not usable even when its capability bits are right.
        let no_format = [node("/dev/video0", STREAMING_CAPTURE, vec![fmt(PixelFormat::Other(1), 640, 480)])];
        assert!(select_device(&no_format).is_none());
    }

    #[test]
    fn plan_stream_sizes_one_buffer_from_the_chosen_format() {
        let devices = [node("/dev/video0", STREAMING_CAPTURE, vec![fmt(PixelFormat::Yuyv, 640, 480)])];
        let plan = plan_stream(&devices, 1280, 720).unwrap();
        assert_eq!(plan.path, "/dev/video0");
        assert_eq!(plan.format, fmt(PixelFormat::Yuyv, 640, 480));
        assert_eq!(plan.buffers, BUFFER_COUNT);
        assert_eq!(plan.buffer_bytes, Some(614_400));
        // A compressed stream has no fixed buffer size.
        let jpeg = [node("/dev/video0", STREAMING_CAPTURE, vec![fmt(PixelFormat::Mjpeg, 1280, 720)])];
        assert_eq!(plan_stream(&jpeg, 1280, 720).unwrap().buffer_bytes, None);
        assert_eq!(plan_stream(&[], 1280, 720), Err(StreamError::NoUsableDevice));
    }
}
