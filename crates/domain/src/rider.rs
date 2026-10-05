//! Rider profile (onboarding answers), FTP history and units.
//! All quantities are stored in SI units; the UI converts for display.

use crate::time::Date;
use crate::workout::Category;
use rl_json::{json_enum, json_struct};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    GeneralFitness,
    Endurance,
    Climbing,
    Event,
    FtpImprovement,
    Returning,
    Maintain,
}
json_enum!(Goal {
    GeneralFitness = "general_fitness",
    Endurance = "endurance",
    Climbing = "climbing",
    Event = "event",
    FtpImprovement = "ftp_improvement",
    Returning = "returning",
    Maintain = "maintain",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Experience {
    New,
    Some,
    Experienced,
}
json_enum!(Experience { New = "new", Some = "some", Experienced = "experienced" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Metric,
    Imperial,
}
json_enum!(Units { Metric = "metric", Imperial = "imperial" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    /// Trainer or power meter measures power.
    Measured,
    /// Trainer estimates power from speed (e.g. a classic trainer + speed sensor).
    Estimated,
    None,
}
json_enum!(PowerSource { Measured = "measured", Estimated = "estimated", None = "none" });

#[derive(Debug, Clone, PartialEq)]
pub struct AiConsent {
    /// Cloud/remote AI requests allowed at all. Local providers on this
    /// computer (e.g. Ollama) still require this to be true, because the app
    /// cannot verify where a configured endpoint really sends data.
    pub enabled: bool,
    pub share_activity_summaries: bool,
    pub share_limitations: bool,
    pub updated_utc: i64,
}
json_struct!(AiConsent {
    enabled: "enabled" = false,
    share_activity_summaries: "share_activity_summaries" = false,
    share_limitations: "share_limitations" = false,
    updated_utc: "updated_utc" = 0,
});

impl Default for AiConsent {
    fn default() -> Self {
        AiConsent { enabled: false, share_activity_summaries: false, share_limitations: false, updated_utc: 0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiderProfile {
    pub schema: u32,
    pub display_name: String,
    pub goal: Goal,
    pub event_date: Option<Date>,
    pub event_name: String,
    pub experience: Experience,
    /// Average minutes per week ridden over the last ~4 weeks.
    pub recent_weekly_min: u32,
    /// Weeks of consistent riding in the last 12 weeks.
    pub consistent_weeks: u32,
    /// Weeks since last regular training (0 = currently training).
    pub weeks_off: u32,
    /// Minutes available per ISO weekday (Mon..Sun); 0 = unavailable/rest.
    pub availability_min: Vec<u32>,
    pub long_ride_day: Option<u8>,
    pub competing_activities: String,
    pub rider_mass_kg: f64,
    pub bike_mass_kg: f64,
    pub units: Units,
    pub age_range: String,
    pub max_hr: Option<u16>,
    pub threshold_hr: Option<u16>,
    pub power_source: PowerSource,
    pub preferred_categories: Vec<Category>,
    pub avoided_categories: Vec<Category>,
    /// Injuries or limitations the rider chose to disclose. Private by default.
    pub limitations: String,
    pub ai_consent: AiConsent,
    /// Trainer difficulty for road simulation, percent (100 = full terrain).
    pub trainer_difficulty_pct: f64,
    pub created_utc: i64,
    pub updated_utc: i64,
}
json_struct!(RiderProfile {
    schema: "schema" = PROFILE_SCHEMA_VERSION,
    display_name: "display_name" = String::new(),
    goal: "goal",
    event_date: "event_date",
    event_name: "event_name" = String::new(),
    experience: "experience",
    recent_weekly_min: "recent_weekly_min",
    consistent_weeks: "consistent_weeks",
    weeks_off: "weeks_off" = 0,
    availability_min: "availability_min",
    long_ride_day: "long_ride_day",
    competing_activities: "competing_activities" = String::new(),
    rider_mass_kg: "rider_mass_kg",
    bike_mass_kg: "bike_mass_kg",
    units: "units" = Units::Metric,
    age_range: "age_range" = String::new(),
    max_hr: "max_hr",
    threshold_hr: "threshold_hr",
    power_source: "power_source" = PowerSource::Measured,
    preferred_categories: "preferred_categories" = Vec::new(),
    avoided_categories: "avoided_categories" = Vec::new(),
    limitations: "limitations" = String::new(),
    ai_consent: "ai_consent" = AiConsent::default(),
    trainer_difficulty_pct: "trainer_difficulty_pct" = 100.0,
    created_utc: "created_utc" = 0,
    updated_utc: "updated_utc" = 0,
});

impl RiderProfile {
    pub fn demo() -> RiderProfile {
        RiderProfile {
            schema: PROFILE_SCHEMA_VERSION,
            display_name: "Demo rider".into(),
            goal: Goal::GeneralFitness,
            event_date: None,
            event_name: String::new(),
            experience: Experience::Some,
            recent_weekly_min: 150,
            consistent_weeks: 6,
            weeks_off: 0,
            availability_min: vec![0, 60, 0, 60, 0, 90, 45],
            long_ride_day: Some(5),
            competing_activities: String::new(),
            rider_mass_kg: 75.0,
            bike_mass_kg: 9.0,
            units: Units::Metric,
            age_range: String::new(),
            max_hr: None,
            threshold_hr: None,
            power_source: PowerSource::Measured,
            preferred_categories: vec![],
            avoided_categories: vec![],
            limitations: String::new(),
            ai_consent: AiConsent::default(),
            trainer_difficulty_pct: 100.0,
            created_utc: 0,
            updated_utc: 0,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.availability_min.len() != 7 {
            return Err("Availability must list 7 days (Mon–Sun).".into());
        }
        if self.availability_min.iter().any(|m| *m > 6 * 60) {
            return Err("Available time per day must be at most 6 hours.".into());
        }
        if self.availability_min.iter().all(|m| *m == 0) {
            return Err("Choose at least one day you can ride.".into());
        }
        if let Some(d) = self.long_ride_day {
            if d > 6 || self.availability_min[d as usize] == 0 {
                return Err("The long-ride day must be one of your available days.".into());
            }
        }
        if !(25.0..=250.0).contains(&self.rider_mass_kg) || !self.rider_mass_kg.is_finite() {
            return Err("Rider mass must be between 25 and 250 kg.".into());
        }
        if !(3.0..=40.0).contains(&self.bike_mass_kg) || !self.bike_mass_kg.is_finite() {
            return Err("Bike mass must be between 3 and 40 kg.".into());
        }
        if self.recent_weekly_min > 40 * 60 {
            return Err("Recent weekly riding time looks too high (max 40 h).".into());
        }
        if self.consistent_weeks > 52 || self.weeks_off > 520 {
            return Err("Training history values are out of range.".into());
        }
        if let Some(h) = self.max_hr {
            if !(100..=230).contains(&h) {
                return Err("Max heart rate must be 100–230 bpm.".into());
            }
        }
        if let Some(h) = self.threshold_hr {
            if !(80..=220).contains(&h) {
                return Err("Threshold heart rate must be 80–220 bpm.".into());
            }
        }
        if !(0.0..=100.0).contains(&self.trainer_difficulty_pct) || !self.trainer_difficulty_pct.is_finite() {
            return Err("Trainer difficulty must be 0–100%.".into());
        }
        for (name, s, max) in [("Name", &self.display_name, 60), ("Event name", &self.event_name, 120), ("Limitations", &self.limitations, 1000), ("Other activities", &self.competing_activities, 300)] {
            if s.chars().count() > max {
                return Err(format!("{name} is too long (max {max} characters)."));
            }
        }
        Ok(())
    }

    pub fn system_mass_kg(&self) -> f64 {
        self.rider_mass_kg + self.bike_mass_kg
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtpMethod {
    /// Rider entered a value they already knew.
    Known,
    RampTest,
    TwentyMinuteTest,
    /// Provisional value derived from an assessment and accepted by the rider.
    Estimate,
    Manual,
}
json_enum!(FtpMethod { Known = "known", RampTest = "ramp_test", TwentyMinuteTest = "twenty_minute_test", Estimate = "estimate", Manual = "manual" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Low,
    Medium,
    High,
}
json_enum!(Confidence { Low = "low", Medium = "medium", High = "high" });

#[derive(Debug, Clone, PartialEq)]
pub struct FtpEntry {
    pub id: String,
    pub watts: f64,
    pub effective: Date,
    pub method: FtpMethod,
    pub confidence: Confidence,
    pub provisional: bool,
    pub note: String,
    pub created_utc: i64,
}
json_struct!(FtpEntry {
    id: "id",
    watts: "watts",
    effective: "effective",
    method: "method",
    confidence: "confidence",
    provisional: "provisional" = false,
    note: "note" = String::new(),
    created_utc: "created_utc" = 0,
});

impl FtpEntry {
    pub fn validate(&self) -> Result<(), String> {
        if !self.watts.is_finite() || !(40.0..=600.0).contains(&self.watts) {
            return Err("FTP must be between 40 and 600 W.".into());
        }
        if self.note.chars().count() > 300 {
            return Err("Note is too long.".into());
        }
        Ok(())
    }
}

/// The FTP in effect on `date`: latest entry with effective date ≤ date.
pub fn ftp_on(history: &[FtpEntry], date: Date) -> Option<&FtpEntry> {
    history.iter().filter(|e| e.effective <= date).max_by(|a, b| a.effective.cmp(&b.effective).then(a.created_utc.cmp(&b.created_utc)))
}

/// Provisional FTP estimates from optional assessments. Conventions are
/// documented in docs/coaching-policy.md; results are always provisional and
/// require the rider's acceptance.
pub fn ftp_from_ramp(best_one_minute_w: f64) -> f64 {
    (best_one_minute_w * 0.75).round()
}
pub fn ftp_from_twenty_minute(avg_w: f64) -> f64 {
    (avg_w * 0.95).round()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_json::{FromJson, ToJson};
    #[test]
    fn profile_roundtrip_and_validation() {
        let p = RiderProfile::demo();
        assert!(p.validate().is_ok());
        let back = RiderProfile::from_json(&rl_json::parse(&p.to_json().to_string_compact()).unwrap()).unwrap();
        assert_eq!(back, p);
        let mut bad = p.clone();
        bad.availability_min = vec![0; 7];
        assert!(bad.validate().is_err());
        let mut bad = p.clone();
        bad.long_ride_day = Some(0); // Monday unavailable
        assert!(bad.validate().is_err());
        let mut bad = p.clone();
        bad.rider_mass_kg = f64::NAN;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn ftp_lookup_uses_effective_date() {
        let mk = |w: f64, d: &str| FtpEntry {
            id: d.into(),
            watts: w,
            effective: Date::parse(d).unwrap(),
            method: FtpMethod::Known,
            confidence: Confidence::Medium,
            provisional: false,
            note: String::new(),
            created_utc: 0,
        };
        let h = vec![mk(200.0, "2026-01-01"), mk(220.0, "2026-06-01")];
        assert_eq!(ftp_on(&h, Date::parse("2026-03-01").unwrap()).unwrap().watts, 200.0);
        assert_eq!(ftp_on(&h, Date::parse("2026-07-01").unwrap()).unwrap().watts, 220.0);
        assert!(ftp_on(&h, Date::parse("2025-07-01").unwrap()).is_none());
        assert_eq!(ftp_from_ramp(300.0), 225.0);
        assert_eq!(ftp_from_twenty_minute(250.0), 238.0);
    }
}
