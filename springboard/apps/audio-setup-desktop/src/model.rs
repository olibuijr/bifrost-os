//! The device model the window shows: what a device is (hardware, aggregate or multi-output), what its formats and
//! timing are, and how those numbers read on screen. Pure data and arithmetic: nothing here talks to the server, so
//! the tests run on the host without a device.

use bifrost_audio_types::{FormatFlags, StreamFormat};

/// The transport string the server reports for an aggregate device (the Audio Setup app creates these).
pub const TRANSPORT_AGGREGATE: &str = "aggregate";
/// The transport string the server reports for a multi-output device (the same mix to several outputs).
pub const TRANSPORT_MULTI_OUTPUT: &str = "multi-output";

/// What a device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A device the driver found (an ALSA PCM, an HDMI or DisplayPort output, the null device).
    Hardware,
    /// Several devices as one, with drift compensation on the ones that are not the clock source.
    Aggregate,
    /// One mix sent to several output devices at the same rate.
    MultiOutput,
}

impl Kind {
    /// The kind a transport string names. Anything the app does not make is hardware.
    pub fn from_transport(transport: &str) -> Kind {
        match transport {
            TRANSPORT_AGGREGATE => Kind::Aggregate,
            TRANSPORT_MULTI_OUTPUT => Kind::MultiOutput,
            _ => Kind::Hardware,
        }
    }

    /// The transport string a kind is created with.
    pub fn transport(self) -> &'static str {
        match self {
            Kind::Hardware => "",
            Kind::Aggregate => TRANSPORT_AGGREGATE,
            Kind::MultiOutput => TRANSPORT_MULTI_OUTPUT,
        }
    }

    /// Aggregates and multi-output devices are made by the user, so they can be edited and removed.
    pub fn is_made(self) -> bool {
        self != Kind::Hardware
    }

    /// The sidebar section the kind is listed under.
    pub fn section(self) -> &'static str {
        if self.is_made() {
            "Aggregates"
        } else {
            "Hardware"
        }
    }
}

/// One device as the window shows it. Every number is what the server reported; a value the server did not give is
/// `None` or zero, and the detail view says so.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    /// The object id the server knows the device by (the sidebar row id is derived from the list index).
    pub object: u32,
    pub uid: String,
    pub name: String,
    pub manufacturer: String,
    pub transport: String,
    pub kind: Kind,
    pub alive: bool,
    /// The nominal sample rate in hertz.
    pub rate: f64,
    /// The output stream's virtual format (f32, non-interleaved on the server); its channel count is `output_channels`.
    pub output_format: Option<StreamFormat>,
    pub input_channels: u32,
    pub output_channels: u32,
    /// The current IO buffer size in frames, and the range the device accepts.
    pub buffer_frames: u32,
    pub buffer_range: (u32, u32),
    /// The device's output latency and safety offset, in frames.
    pub latency_frames: u32,
    pub safety_frames: u32,
    /// The main output volume, 0.0 to 1.0, when the device has one.
    pub volume: Option<f32>,
}

impl DeviceInfo {
    /// A device with the given identity and nothing else reported yet.
    pub fn new(object: u32, uid: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            object,
            uid: uid.to_string(),
            name: name.to_string(),
            manufacturer: String::new(),
            transport: String::new(),
            kind: Kind::Hardware,
            alive: true,
            rate: 0.0,
            output_format: None,
            input_channels: 0,
            output_channels: 0,
            buffer_frames: 0,
            buffer_range: (0, 0),
            latency_frames: 0,
            safety_frames: 0,
            volume: None,
        }
    }

    /// Whether the device has an output side (a multi-output member must have one).
    pub fn has_output(&self) -> bool {
        self.output_channels > 0
    }
}

/// Seconds in frames at a rate, in milliseconds (`frames / rate`). A rate of zero (not reported) gives zero.
pub fn frames_to_ms(frames: u32, rate: f64) -> f64 {
    if rate <= 0.0 {
        return 0.0;
    }
    f64::from(frames) * 1000.0 / rate
}

/// A sample rate as the sidebar and the detail view write it: `48 kHz`, `44.1 kHz`, `0 Hz` when unknown.
pub fn format_rate(rate: f64) -> String {
    if rate <= 0.0 {
        return "Unknown".to_string();
    }
    if rate >= 1000.0 && (rate % 1000.0).abs() < 1e-6 {
        format!("{} kHz", (rate / 1000.0).round() as u64)
    } else if rate >= 1000.0 {
        format!("{:.1} kHz", rate / 1000.0)
    } else {
        format!("{} Hz", rate.round() as u64)
    }
}

/// A stream format in words: `32-bit float, 2 channels, non-interleaved`.
pub fn format_stream(format: &StreamFormat) -> String {
    let kind = if format.flags.contains(FormatFlags::FLOAT) {
        "float"
    } else {
        "integer"
    };
    let layout = if format.is_interleaved() {
        "interleaved"
    } else {
        "non-interleaved"
    };
    let channels = if format.channels == 1 {
        "1 channel".to_string()
    } else {
        format!("{} channels", format.channels)
    };
    format!("{}-bit {}, {}, {}", format.bits_per_channel, kind, channels, layout)
}

/// A buffer size with its time: `256 frames (5.3 ms)`.
pub fn format_buffer(frames: u32, rate: f64) -> String {
    if frames == 0 {
        return "Unknown".to_string();
    }
    format!("{} frames ({:.1} ms)", frames, frames_to_ms(frames, rate))
}

/// A latency with its time: `64 frames (1.3 ms)`.
pub fn format_latency(frames: u32, rate: f64) -> String {
    format!("{} frames ({:.1} ms)", frames, frames_to_ms(frames, rate))
}

/// A volume as a percentage, rounded: `0.456` reads `46 %`.
pub fn format_volume(volume: f32) -> String {
    let pct = (f64::from(volume.clamp(0.0, 1.0)) * 100.0).round() as u32;
    format!("{} %", pct)
}

/// The channel summary: `2 in, 2 out`, or `none` when the device has no channels.
pub fn format_channels(input: u32, output: u32) -> String {
    match (input, output) {
        (0, 0) => "None".to_string(),
        (i, 0) => format!("{i} in"),
        (0, o) => format!("{o} out"),
        (i, o) => format!("{i} in, {o} out"),
    }
}

/// The one-line description under a device name: manufacturer and transport, dropping what is missing.
pub fn subtitle(info: &DeviceInfo) -> String {
    let parts: Vec<&str> = [info.manufacturer.as_str(), info.transport.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        format_channels(info.input_channels, info.output_channels)
    } else {
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::FormatId;

    #[test]
    fn kind_follows_the_transport_string() {
        assert_eq!(Kind::from_transport("aggregate"), Kind::Aggregate);
        assert_eq!(Kind::from_transport("multi-output"), Kind::MultiOutput);
        assert_eq!(Kind::from_transport("pci"), Kind::Hardware);
        assert_eq!(Kind::from_transport(""), Kind::Hardware);
        assert_eq!(Kind::Aggregate.transport(), "aggregate");
        assert!(Kind::MultiOutput.is_made());
        assert!(!Kind::Hardware.is_made());
        assert_eq!(Kind::Hardware.section(), "Hardware");
        assert_eq!(Kind::Aggregate.section(), "Aggregates");
    }

    #[test]
    fn latency_in_milliseconds_is_frames_over_rate() {
        // 64 frames at 48 kHz is 1.333 ms; 256 frames is 5.333 ms; 1024 frames is 21.333 ms (docs/audio-architecture.md)
        assert!((frames_to_ms(64, 48000.0) - 1.3333).abs() < 1e-3);
        assert!((frames_to_ms(256, 48000.0) - 5.3333).abs() < 1e-3);
        assert!((frames_to_ms(1024, 48000.0) - 21.3333).abs() < 1e-3);
        // 480 frames at 44.1 kHz is 10.884 ms
        assert!((frames_to_ms(480, 44100.0) - 10.8843).abs() < 1e-3);
        assert_eq!(frames_to_ms(64, 0.0), 0.0);
    }

    #[test]
    fn rates_read_as_kilohertz_or_hertz() {
        assert_eq!(format_rate(48000.0), "48 kHz");
        assert_eq!(format_rate(96000.0), "96 kHz");
        assert_eq!(format_rate(44100.0), "44.1 kHz");
        assert_eq!(format_rate(8000.0), "8 kHz");
        assert_eq!(format_rate(440.0), "440 Hz");
        assert_eq!(format_rate(0.0), "Unknown");
    }

    #[test]
    fn stream_format_reads_in_words() {
        let f = StreamFormat::float32(48000.0, 2);
        assert_eq!(format_stream(&f), "32-bit float, 2 channels, non-interleaved");
        let mono = StreamFormat::integer(48000.0, 1, 16, 2);
        assert_eq!(format_stream(&mono), "16-bit integer, 1 channel, interleaved");
        assert_eq!(mono.format, FormatId::LinearPcm);
    }

    #[test]
    fn buffers_and_latencies_carry_their_time() {
        assert_eq!(format_buffer(256, 48000.0), "256 frames (5.3 ms)");
        assert_eq!(format_buffer(64, 48000.0), "64 frames (1.3 ms)");
        assert_eq!(format_buffer(0, 48000.0), "Unknown");
        assert_eq!(format_latency(512, 48000.0), "512 frames (10.7 ms)");
    }

    #[test]
    fn volume_rounds_to_a_percentage_and_clamps() {
        assert_eq!(format_volume(0.456), "46 %");
        assert_eq!(format_volume(0.0), "0 %");
        assert_eq!(format_volume(1.0), "100 %");
        assert_eq!(format_volume(1.7), "100 %");
        assert_eq!(format_volume(-0.2), "0 %");
    }

    #[test]
    fn channel_summary_and_subtitle() {
        assert_eq!(format_channels(0, 0), "None");
        assert_eq!(format_channels(2, 0), "2 in");
        assert_eq!(format_channels(0, 2), "2 out");
        assert_eq!(format_channels(2, 2), "2 in, 2 out");
        let mut d = DeviceInfo::new(7, "hw:0,0", "Speakers");
        d.output_channels = 2;
        assert_eq!(subtitle(&d), "2 out");
        d.manufacturer = "Intel".into();
        d.transport = "PCI".into();
        assert_eq!(subtitle(&d), "Intel · PCI");
    }
}
