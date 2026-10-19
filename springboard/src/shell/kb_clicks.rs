//! bifrost keyboard click sounds: a letter click, a lower delete click and a modifier click.
//! BifrOSt ships no recorded keyboard sounds, so they are synthesised as short decaying bursts.
//! The keyboard only queues the kind; the shell plays it, and only when
//! Settings > Sounds > Keyboard Clicks is on and the device is not muted.

pub const RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Key,
    Delete,
    Modifier,
}

/// Pure gate: play only with the setting on and the output not muted.
pub fn should_play(setting_on: bool, muted: bool) -> bool {
    setting_on && !muted
}

/// Mono i16 PCM for one click: (frequency Hz, length ms, peak).
pub fn pcm(kind: Click, volume: f32) -> Vec<i16> {
    let (freq, ms, peak) = match kind {
        Click::Key => (1800.0f32, 14.0f32, 0.30f32),
        Click::Delete => (1100.0, 16.0, 0.30),
        Click::Modifier => (1400.0, 14.0, 0.26),
    };
    let n = (RATE as f32 * ms / 1000.0) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let env = (-(i as f32) / (n as f32 * 0.25)).exp() * (1.0 - i as f32 / n as f32);
            let tone = (t * freq * std::f32::consts::TAU).sin();
            // a little noise at the attack gives the tick
            let noise = ((i as u32).wrapping_mul(2654435761) >> 16) as f32 / 32768.0 - 1.0;
            let s = (tone * 0.8 + noise * 0.2 * (1.0 - i as f32 / n as f32)) * env * peak * volume.clamp(0.0, 1.0);
            (s * i16::MAX as f32) as i16
        })
        .collect()
}

/// Clicks waiting to be played by the shell.
#[derive(Default)]
pub struct Clicks {
    pending: Vec<Click>,
}

impl Clicks {
    pub fn push(&mut self, c: Click) {
        if self.pending.len() < 8 {
            self.pending.push(c);
        }
    }
    pub fn take(&mut self) -> Vec<Click> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gate_and_pcm() {
        assert!(should_play(true, false));
        assert!(!should_play(false, false) && !should_play(true, true));
        assert!(!pcm(Click::Key, 1.0).is_empty());
        assert!(pcm(Click::Delete, 0.0).iter().all(|&s| s == 0));
    }
}
