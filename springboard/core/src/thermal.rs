//! Thermal state model: `ProcessInfo.ThermalState` (Nominal, Fair, Serious, Critical) read from the kernel's thermal
//! zones. Pure: `thermal_state_for` takes a zone temperature and its trip points; `worst_in` reads one class
//! directory (`/sys/class/thermal` on the device) and reports the worst zone. The SDK (`bifrost_app::process_info`)
//! and the shell's `power` service both use this model.
use std::path::Path;

/// `ProcessInfo.ThermalState`, ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
}

impl ThermalState {
    /// The wire name used by the shell's `power` service: `nominal`, `fair`, `serious`, `critical`.
    pub fn name(self) -> &'static str {
        match self {
            ThermalState::Nominal => "nominal",
            ThermalState::Fair => "fair",
            ThermalState::Serious => "serious",
            ThermalState::Critical => "critical",
        }
    }

    /// The state for a wire name; None for anything else.
    pub fn parse(text: &str) -> Option<ThermalState> {
        match text.trim() {
            "nominal" => Some(ThermalState::Nominal),
            "fair" => Some(ThermalState::Fair),
            "serious" => Some(ThermalState::Serious),
            "critical" => Some(ThermalState::Critical),
            _ => None,
        }
    }
}

/// One kernel trip point of a thermal zone: `kind` is the `trip_point_N_type` text, `temp` is millidegrees C.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TripPoint {
    pub kind: String,
    pub temp: i64,
}

/// The state of one zone: the `passive` trip reached is Fair, `hot` Serious, 5 C below `critical` Critical. Zones
/// without such trips use fixed limits (75, 85 and 95 C).
pub fn thermal_state_for(temp: i64, trips: &[TripPoint]) -> ThermalState {
    let find = |kind: &str| trips.iter().filter(|t| t.kind == kind && t.temp > 0).map(|t| t.temp).min();
    let known = trips.iter().any(|t| matches!(t.kind.as_str(), "passive" | "hot" | "critical"));
    let (fair, serious, critical) = if known {
        let critical = find("critical");
        (find("passive"), find("hot").or(critical.map(|c| c - 10_000)), critical.map(|c| c - 5_000))
    } else {
        (Some(75_000), Some(85_000), Some(95_000))
    };
    if critical.is_some_and(|c| temp >= c) {
        ThermalState::Critical
    } else if serious.is_some_and(|c| temp >= c) {
        ThermalState::Serious
    } else if fair.is_some_and(|c| temp >= c) {
        ThermalState::Fair
    } else {
        ThermalState::Nominal
    }
}

/// The worst state over every `thermal_zone*` under `class_dir`. A missing directory or an unreadable zone counts as
/// Nominal (the zone is skipped), so the answer never fails.
pub fn worst_in(class_dir: &Path) -> ThermalState {
    let Ok(dir) = std::fs::read_dir(class_dir) else { return ThermalState::Nominal };
    let num = |p: &Path| std::fs::read_to_string(p).ok().and_then(|t| t.trim().parse::<i64>().ok());
    let mut worst = ThermalState::Nominal;
    for e in dir.flatten() {
        if !e.file_name().to_string_lossy().starts_with("thermal_zone") {
            continue;
        }
        let zone = e.path();
        let Some(temp) = num(&zone.join("temp")) else { continue };
        let mut trips = Vec::new();
        for i in 0..16 {
            let Some(t) = num(&zone.join(format!("trip_point_{i}_temp"))) else { break };
            let kind = std::fs::read_to_string(zone.join(format!("trip_point_{i}_type"))).unwrap_or_default().trim().to_owned();
            trips.push(TripPoint { kind, temp: t });
        }
        worst = worst.max(thermal_state_for(temp, &trips));
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_state_follows_the_trip_points() {
        let trips = vec![
            TripPoint { kind: "passive".into(), temp: 80_000 },
            TripPoint { kind: "hot".into(), temp: 90_000 },
            TripPoint { kind: "critical".into(), temp: 100_000 },
        ];
        assert_eq!(thermal_state_for(50_000, &trips), ThermalState::Nominal);
        assert_eq!(thermal_state_for(80_000, &trips), ThermalState::Fair);
        assert_eq!(thermal_state_for(90_000, &trips), ThermalState::Serious);
        assert_eq!(thermal_state_for(95_000, &trips), ThermalState::Critical);
        assert_eq!(thermal_state_for(70_000, &[]), ThermalState::Nominal);
        assert_eq!(thermal_state_for(96_000, &[]), ThermalState::Critical);
        assert!(ThermalState::Fair < ThermalState::Serious);
    }

    #[test]
    fn wire_names_round_trip() {
        for s in [ThermalState::Nominal, ThermalState::Fair, ThermalState::Serious, ThermalState::Critical] {
            assert_eq!(ThermalState::parse(s.name()), Some(s));
        }
        assert_eq!(ThermalState::parse(" fair \n"), Some(ThermalState::Fair));
        assert_eq!(ThermalState::parse("hot"), None);
    }

    #[test]
    fn worst_zone_wins_and_a_missing_directory_is_nominal() {
        let dir = std::env::temp_dir().join(format!("bifrost-thermal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let hot = dir.join("thermal_zone0");
        let cool = dir.join("thermal_zone1");
        std::fs::create_dir_all(&hot).unwrap();
        std::fs::create_dir_all(&cool).unwrap();
        std::fs::write(hot.join("temp"), "96000\n").unwrap();
        std::fs::write(cool.join("temp"), "40000\n").unwrap();
        std::fs::create_dir_all(dir.join("cooling_device0")).unwrap();
        assert_eq!(worst_in(&dir), ThermalState::Critical);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(worst_in(Path::new("/nonexistent/bifrost-thermal-class")), ThermalState::Nominal);
    }
}
