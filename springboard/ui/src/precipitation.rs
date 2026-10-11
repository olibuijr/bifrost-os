//! Precipitation (`Precipitation`): the form of precipitation, the amounts by form (`PrecipitationAmountByType`), the
//! snowfall amount (`SnowfallAmount`) and the next hour minute by minute (`MinuteForecast`, `MinuteWeather`). These mirror
//! the precipitation structures of the official weather framework. Amounts are millimetres of liquid equivalent per hour
//! or per period, snowfall is centimetres, the chance is a fraction from 0 to 1, and times are Unix seconds.
//! Every constructor clamps bad input (negative, NaN, infinite) to a safe value, so a broken data feed cannot panic a
//! caller or show a negative amount.

use std::fmt::Write as _;

/// The form of precipitation. `None` means the sky is dry at that time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Precipitation {
    None,
    Rain,
    Snow,
    Sleet,
    Hail,
    Mixed,
}

impl Precipitation {
    /// A short localisable name, as the official framework gives in its description property.
    pub fn description(self) -> &'static str {
        match self {
            Precipitation::None => "No precipitation",
            Precipitation::Rain => "Rain",
            Precipitation::Snow => "Snow",
            Precipitation::Sleet => "Sleet",
            Precipitation::Hail => "Hail",
            Precipitation::Mixed => "Wintry mix",
        }
    }

    /// True for every form except `None`.
    pub fn is_falling(self) -> bool {
        self != Precipitation::None
    }

    /// True when the precipitation is frozen (snow, sleet, hail or a wintry mix).
    pub fn is_frozen(self) -> bool {
        matches!(self, Precipitation::Snow | Precipitation::Sleet | Precipitation::Hail | Precipitation::Mixed)
    }
}

/// Clamps a measurement to a finite, non-negative value.
fn non_negative(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// Amounts for each form of precipitation, in millimetres of liquid equivalent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PrecipitationAmountByType {
    pub rain_mm: f64,
    pub snow_mm: f64,
    pub sleet_mm: f64,
    pub hail_mm: f64,
}

impl PrecipitationAmountByType {
    /// Builds the amounts, clamping each one to zero when it is negative or not finite.
    pub fn new(rain_mm: f64, snow_mm: f64, sleet_mm: f64, hail_mm: f64) -> Self {
        PrecipitationAmountByType {
            rain_mm: non_negative(rain_mm),
            snow_mm: non_negative(snow_mm),
            sleet_mm: non_negative(sleet_mm),
            hail_mm: non_negative(hail_mm),
        }
    }

    /// The sum of all forms.
    pub fn total_mm(&self) -> f64 {
        self.rain_mm + self.snow_mm + self.sleet_mm + self.hail_mm
    }

    /// The form with the largest amount. Ties go to the earlier form in the order rain, snow, sleet, hail,
    /// and `None` when nothing falls.
    pub fn dominant(&self) -> Precipitation {
        let forms = [
            (self.rain_mm, Precipitation::Rain),
            (self.snow_mm, Precipitation::Snow),
            (self.sleet_mm, Precipitation::Sleet),
            (self.hail_mm, Precipitation::Hail),
        ];
        let mut best = (0.0, Precipitation::None);
        for (amount, form) in forms {
            if amount > best.0 {
                best = (amount, form);
            }
        }
        best.1
    }
}

/// Snowfall in centimetres of snow on the ground.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SnowfallAmount {
    pub centimetres: f64,
}

impl SnowfallAmount {
    /// Clamps a negative or non-finite value to zero.
    pub fn new(centimetres: f64) -> Self {
        SnowfallAmount { centimetres: non_negative(centimetres) }
    }

    /// True when any snow fell.
    pub fn is_measurable(&self) -> bool {
        self.centimetres > 0.0
    }

    /// The amount as text with one decimal, for example `"2.5 cm"`.
    pub fn text(&self) -> String {
        format!("{:.1} cm", self.centimetres)
    }
}

/// The precipitation for one minute of the next hour (`MinuteWeather`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinuteWeather {
    /// The start of the minute, in Unix seconds.
    pub start: i64,
    pub precipitation: Precipitation,
    /// Probability of precipitation in this minute, from 0 to 1.
    pub chance: f64,
    /// Forecast intensity in millimetres per hour.
    pub intensity_mm_h: f64,
}

impl MinuteWeather {
    /// Builds one minute. The chance is clamped to 0..=1; the intensity is clamped to zero or above.
    pub fn new(start: i64, precipitation: Precipitation, chance: f64, intensity_mm_h: f64) -> Self {
        let chance = if chance.is_finite() { chance.clamp(0.0, 1.0) } else { 0.0 };
        MinuteWeather { start, precipitation, chance, intensity_mm_h: non_negative(intensity_mm_h) }
    }

    /// The chance as a whole percentage, rounded half up.
    pub fn chance_percent(&self) -> u8 {
        (self.chance * 100.0 + 0.5).floor() as u8
    }
}

/// The minute forecast for the next hour, kept in time order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MinuteForecast {
    minutes: Vec<MinuteWeather>,
}

impl MinuteForecast {
    /// Sorts the minutes by start time. Entries with the same start keep their order.
    pub fn new(mut minutes: Vec<MinuteWeather>) -> Self {
        minutes.sort_by_key(|m| m.start);
        MinuteForecast { minutes }
    }

    pub fn minutes(&self) -> &[MinuteWeather] {
        &self.minutes
    }

    pub fn is_empty(&self) -> bool {
        self.minutes.is_empty()
    }

    /// The minute that covers `now`: the last minute that starts at or before it. Before the first minute the first
    /// minute is returned, so a slightly stale forecast still answers.
    pub fn current(&self, now: i64) -> Option<&MinuteWeather> {
        let idx = self.minutes.partition_point(|m| m.start <= now);
        if idx == 0 {
            self.minutes.first()
        } else {
            self.minutes.get(idx - 1)
        }
    }

    /// The first later minute whose form differs from the current one, and that form, as the start of the change.
    pub fn next_change(&self, now: i64) -> Option<(i64, Precipitation)> {
        let current = self.current(now)?.precipitation;
        self.minutes
            .iter()
            .filter(|m| m.start > now)
            .find(|m| m.precipitation != current)
            .map(|m| (m.start, m.precipitation))
    }

    /// The minute with the highest intensity, or `None` when the forecast is empty or dry.
    pub fn peak(&self) -> Option<&MinuteWeather> {
        self.minutes
            .iter()
            .filter(|m| m.intensity_mm_h > 0.0)
            .max_by(|a, b| a.intensity_mm_h.total_cmp(&b.intensity_mm_h))
    }

    /// One sentence for the next hour, as the shell shows it: "Rain starts in 12 min", "Rain stops in 8 min",
    /// "Rain changes to snow in 20 min", "Rain for the next hour" or "No precipitation in the next hour".
    pub fn summary(&self, now: i64) -> String {
        let Some(current) = self.current(now).map(|m| m.precipitation) else {
            return "No minute forecast".to_string();
        };
        let mut out = String::new();
        match (current.is_falling(), self.next_change(now)) {
            (false, None) => out.push_str("No precipitation in the next hour"),
            (false, Some((at, form))) => {
                let _ = write!(out, "{} starts in {}", form.description(), minutes_until(now, at));
            }
            (true, None) => {
                let _ = write!(out, "{} for the next hour", current.description());
            }
            (true, Some((at, form))) if form.is_falling() => {
                let _ = write!(
                    out,
                    "{} changes to {} in {}",
                    current.description(),
                    form.description().to_lowercase(),
                    minutes_until(now, at)
                );
            }
            (true, Some((at, _))) => {
                let _ = write!(out, "{} stops in {}", current.description(), minutes_until(now, at));
            }
        }
        out
    }
}

/// Whole minutes from `now` until `at`, rounded up and at least one, as "N min".
fn minutes_until(now: i64, at: i64) -> String {
    let secs = (at - now).max(0);
    let mins = ((secs + 59) / 60).max(1);
    format!("{mins} min")
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000;

    fn minute(i: i64, p: Precipitation) -> MinuteWeather {
        MinuteWeather::new(T0 + i * 60, p, 0.5, if p.is_falling() { 2.0 } else { 0.0 })
    }

    #[test]
    fn amounts_clamp_bad_input_and_pick_the_dominant_form() {
        let amounts = PrecipitationAmountByType::new(1.5, -3.0, f64::NAN, 0.25);
        assert_eq!(amounts.snow_mm, 0.0);
        assert_eq!(amounts.sleet_mm, 0.0);
        assert!((amounts.total_mm() - 1.75).abs() < 1e-9);
        assert_eq!(amounts.dominant(), Precipitation::Rain);
        assert_eq!(PrecipitationAmountByType::default().dominant(), Precipitation::None);
        // A tie keeps the earlier form in the order rain, snow, sleet, hail.
        assert_eq!(PrecipitationAmountByType::new(0.0, 2.0, 2.0, 0.0).dominant(), Precipitation::Snow);
    }

    #[test]
    fn form_predicates_match_the_description() {
        assert!(!Precipitation::None.is_falling());
        assert!(Precipitation::Rain.is_falling() && !Precipitation::Rain.is_frozen());
        assert!(Precipitation::Mixed.is_frozen() && Precipitation::Sleet.is_frozen());
        assert_eq!(Precipitation::Mixed.description(), "Wintry mix");
    }

    #[test]
    fn snowfall_is_non_negative_and_formats_one_decimal() {
        assert!(!SnowfallAmount::new(-1.0).is_measurable());
        let snow = SnowfallAmount::new(2.46);
        assert!(snow.is_measurable());
        assert_eq!(snow.text(), "2.5 cm");
    }

    #[test]
    fn minute_chance_is_clamped_and_rounds_half_up() {
        assert_eq!(MinuteWeather::new(0, Precipitation::Rain, 1.7, 1.0).chance, 1.0);
        assert_eq!(MinuteWeather::new(0, Precipitation::Rain, f64::NAN, 1.0).chance, 0.0);
        assert_eq!(MinuteWeather::new(0, Precipitation::Rain, 0.125, 1.0).chance_percent(), 13);
        assert_eq!(MinuteWeather::new(0, Precipitation::Rain, 0.5, -4.0).intensity_mm_h, 0.0);
    }

    #[test]
    fn forecast_is_sorted_and_current_covers_now() {
        let f = MinuteForecast::new(vec![minute(2, Precipitation::Rain), minute(0, Precipitation::None)]);
        assert_eq!(f.minutes()[0].start, T0);
        assert_eq!(f.current(T0 + 90).unwrap().start, T0);
        assert_eq!(f.current(T0 + 150).unwrap().start, T0 + 120);
        // Before the first minute the first minute still answers.
        assert_eq!(f.current(T0 - 500).unwrap().start, T0);
        assert!(MinuteForecast::default().current(T0).is_none());
    }

    #[test]
    fn next_change_and_peak_find_the_transition() {
        let mut minutes: Vec<MinuteWeather> = (0..10).map(|i| minute(i, Precipitation::None)).collect();
        minutes.extend((10..20).map(|i| minute(i, Precipitation::Rain)));
        let f = MinuteForecast::new(minutes);
        assert_eq!(f.next_change(T0 + 60), Some((T0 + 600, Precipitation::Rain)));
        assert_eq!(f.next_change(T0 + 700), None);
        assert!(f.peak().is_some());
        assert!(MinuteForecast::new(vec![minute(0, Precipitation::None)]).peak().is_none());
    }

    #[test]
    fn summary_describes_starts_stops_and_dry_hours() {
        let dry_then_rain: Vec<MinuteWeather> = (0..10)
            .map(|i| minute(i, Precipitation::None))
            .chain((10..60).map(|i| minute(i, Precipitation::Rain)))
            .collect();
        assert_eq!(MinuteForecast::new(dry_then_rain.clone()).summary(T0 + 60), "Rain starts in 9 min");

        let rain_then_dry: Vec<MinuteWeather> = (0..5)
            .map(|i| minute(i, Precipitation::Rain))
            .chain((5..60).map(|i| minute(i, Precipitation::None)))
            .collect();
        assert_eq!(MinuteForecast::new(rain_then_dry).summary(T0), "Rain stops in 5 min");

        let dry: Vec<MinuteWeather> = (0..60).map(|i| minute(i, Precipitation::None)).collect();
        assert_eq!(MinuteForecast::new(dry).summary(T0), "No precipitation in the next hour");
        assert_eq!(MinuteForecast::default().summary(T0), "No minute forecast");
    }
}
