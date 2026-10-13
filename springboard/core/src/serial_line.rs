//! Serial line settings: the framing of an asynchronous serial port (std only).
//!
//! A serial port carries each character as a start bit, the data bits (5 to 8), an optional parity
//! bit, and one or two stop bits. This module validates the settings a port open call takes (the
//! standard baud rates, the data width, the parity, the stop bits and the flow control), and works
//! out the wire timing from them: bits per character, characters per second, and how long a buffer
//! takes to send. It opens no device; the caller applies the settings to the port it owns.

/// The standard baud rates a serial port driver offers, in ascending order.
pub const STANDARD_BAUD_RATES: [u32; 20] = [
    50, 75, 110, 134, 150, 200, 300, 600, 1200, 1800, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800,
    921600,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopBits {
    One,
    Two,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowControl {
    None,
    /// RTS and CTS lines.
    Hardware,
    /// XON and XOFF characters in the data stream.
    Software,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineError {
    /// The baud rate is not one of `STANDARD_BAUD_RATES`.
    NonStandardBaud(u32),
    /// The data width is outside 5..=8.
    DataBits(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSettings {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow: FlowControl,
}

impl LineSettings {
    /// Check and build a line configuration.
    pub fn new(
        baud: u32,
        data_bits: u8,
        parity: Parity,
        stop_bits: StopBits,
        flow: FlowControl,
    ) -> Result<LineSettings, LineError> {
        let settings = LineSettings { baud, data_bits, parity, stop_bits, flow };
        settings.validate()?;
        Ok(settings)
    }

    /// The common 8 data bits, no parity, one stop bit, no flow control at the given rate.
    pub fn eight_n_one(baud: u32) -> Result<LineSettings, LineError> {
        LineSettings::new(baud, 8, Parity::None, StopBits::One, FlowControl::None)
    }

    pub fn validate(&self) -> Result<(), LineError> {
        if !STANDARD_BAUD_RATES.contains(&self.baud) {
            return Err(LineError::NonStandardBaud(self.baud));
        }
        if !(5..=8).contains(&self.data_bits) {
            return Err(LineError::DataBits(self.data_bits));
        }
        Ok(())
    }

    /// Bits on the wire for one character: start, data, the parity bit when there is one, and the stop bits.
    pub fn frame_bits(&self) -> u32 {
        let parity = if self.parity == Parity::None { 0 } else { 1 };
        let stop = match self.stop_bits {
            StopBits::One => 1,
            StopBits::Two => 2,
        };
        1 + self.data_bits as u32 + parity + stop
    }

    /// Whole characters per second at the configured rate.
    pub fn characters_per_second(&self) -> u32 {
        self.baud / self.frame_bits()
    }

    /// Microseconds to send `bytes` characters, rounded up to the next microsecond.
    pub fn transmit_micros(&self, bytes: usize) -> u64 {
        let bits = bytes as u64 * self.frame_bits() as u64 * 1_000_000;
        let baud = self.baud as u64;
        bits.div_ceil(baud)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_n_one_is_ten_bits_per_character() {
        let s = LineSettings::eight_n_one(115200).unwrap();
        assert_eq!(s.frame_bits(), 10);
        assert_eq!(s.characters_per_second(), 11520);
    }

    #[test]
    fn parity_and_two_stop_bits_add_to_the_frame() {
        // 1 start + 7 data + 1 parity + 2 stop = 11 bits. 9600 / 11 = 872.7, so 872 characters.
        let s = LineSettings::new(9600, 7, Parity::Odd, StopBits::Two, FlowControl::Hardware).unwrap();
        assert_eq!(s.frame_bits(), 11);
        assert_eq!(s.characters_per_second(), 872);
        // 8 data, even parity, one stop: 11 bits, one more than 8N1.
        let even = LineSettings::new(9600, 8, Parity::Even, StopBits::One, FlowControl::None).unwrap();
        assert_eq!(even.frame_bits(), 11);
    }

    #[test]
    fn validate_rejects_unknown_rates_and_bad_widths() {
        assert_eq!(
            LineSettings::new(12345, 8, Parity::None, StopBits::One, FlowControl::None),
            Err(LineError::NonStandardBaud(12345))
        );
        assert_eq!(LineSettings::eight_n_one(0), Err(LineError::NonStandardBaud(0)));
        assert_eq!(
            LineSettings::new(9600, 4, Parity::None, StopBits::One, FlowControl::None),
            Err(LineError::DataBits(4))
        );
        assert_eq!(
            LineSettings::new(9600, 9, Parity::None, StopBits::One, FlowControl::None),
            Err(LineError::DataBits(9))
        );
    }

    #[test]
    fn transmit_time_rounds_up_to_the_microsecond() {
        // One 8N1 byte at 9600 baud: 10 bits take 1041.67 microseconds, so 1042.
        let slow = LineSettings::eight_n_one(9600).unwrap();
        assert_eq!(slow.transmit_micros(1), 1042);
        assert_eq!(slow.transmit_micros(0), 0);
        // 100 bytes at 115200 baud: 1000 bits take 8680.56 microseconds, so 8681.
        let fast = LineSettings::eight_n_one(115200).unwrap();
        assert_eq!(fast.transmit_micros(100), 8681);
    }

    #[test]
    fn standard_rates_are_strictly_ascending_and_include_the_common_ones() {
        assert!(STANDARD_BAUD_RATES.windows(2).all(|w| w[0] < w[1]));
        assert!(STANDARD_BAUD_RATES.contains(&9600));
        assert!(STANDARD_BAUD_RATES.contains(&115200));
        assert_eq!(STANDARD_BAUD_RATES[STANDARD_BAUD_RATES.len() - 1], 921600);
    }

    #[test]
    fn seven_data_bits_without_parity_is_nine_bits_on_the_wire() {
        let s = LineSettings::new(300, 7, Parity::None, StopBits::One, FlowControl::Software).unwrap();
        assert_eq!(s.frame_bits(), 9);
        assert_eq!(s.characters_per_second(), 33);
    }
}
