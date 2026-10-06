//! Application settings (non-secret). Secrets (API keys) live in the OS
//! credential store via [`crate::Platform`], never in this file or in logs.

use rl_domain::rider::Units;
use rl_json::{json_struct, Value};

pub const SETTINGS_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct AiSettings {
    /// "offline" (rules + deterministic planner only), "local" (model server on
    /// this computer, e.g. Ollama), "remote" (owner-configured endpoint; off
    /// by default and never used unless `remote_enabled`).
    pub provider: String,
    pub preset: String,
    pub base_url: String,
    pub model: String,
    pub remote_enabled: bool,
    pub max_requests_per_day: u32,
    pub timeout_s: u32,
    pub max_tokens: u32,
}
json_struct!(AiSettings {
    provider: "provider" = "offline".to_string(),
    preset: "preset" = "ollama".to_string(),
    base_url: "base_url" = "http://localhost:11434/v1".to_string(),
    model: "model" = "llama3.2".to_string(),
    remote_enabled: "remote_enabled" = false,
    max_requests_per_day: "max_requests_per_day" = 100,
    timeout_s: "timeout_s" = 180,
    max_tokens: "max_tokens" = 1800,
});

#[derive(Debug, Clone, PartialEq)]
pub struct MapSettings {
    pub enabled: bool,
    pub style_url: String,
    pub attribution: String,
}
json_struct!(MapSettings {
    enabled: "enabled" = true,
    style_url: "style_url" = "https://tiles.openfreemap.org/styles/liberty".to_string(),
    attribution: "attribution" = "OpenFreeMap © OpenMapTiles Data from OpenStreetMap".to_string(),
});

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSettings {
    pub routing_enabled: bool,
    pub routing_url: String,
    pub elevation_enabled: bool,
    pub elevation_url: String,
}
json_struct!(ProviderSettings {
    routing_enabled: "routing_enabled" = true,
    routing_url: "routing_url" = "https://valhalla1.openstreetmap.de".to_string(),
    elevation_enabled: "elevation_enabled" = true,
    elevation_url: "elevation_url" = "https://api.open-meteo.com".to_string(),
});

#[derive(Debug, Clone, PartialEq)]
pub struct TrainerSettings {
    pub difficulty_pct: f64,
    pub lookahead_m: f64,
    pub grade_min_pct: f64,
    pub grade_max_pct: f64,
    pub slew_pct_per_s: f64,
    pub stale_ms: u64,
    pub target_interval_ms: u64,
    pub ack_timeout_ms: u64,
}
json_struct!(TrainerSettings {
    difficulty_pct: "difficulty_pct" = 100.0,
    lookahead_m: "lookahead_m" = 0.0,
    grade_min_pct: "grade_min_pct" = -10.0,
    grade_max_pct: "grade_max_pct" = 20.0,
    slew_pct_per_s: "slew_pct_per_s" = 1.5,
    stale_ms: "stale_ms" = 3000,
    target_interval_ms: "target_interval_ms" = 1000,
    ack_timeout_ms: "ack_timeout_ms" = 3000,
});

/// Ride behaviour that isn't trainer control (so it may change mid-ride).
#[derive(Debug, Clone, PartialEq)]
pub struct RideSettings {
    /// Pause when you stop pedalling, resume when you start again.
    pub auto_pause: bool,
}
json_struct!(RideSettings {
    auto_pause: "auto_pause" = true,
});

/// The coach during rides.
#[derive(Debug, Clone, PartialEq)]
pub struct RideCoachSettings {
    /// Rule-based ride cues (interval previews, cadence, climbs). Offline.
    pub cues: bool,
    /// When the AI coach is on: comment at key moments (hard interval
    /// starts, halfway, last interval, long climbs), at most every
    /// `moment_gap_s` seconds.
    pub ai_moments: bool,
    pub moment_gap_s: u32,
    /// Read cues and replies aloud with the operating system's voice.
    pub voice: bool,
}
json_struct!(RideCoachSettings {
    cues: "cues" = true,
    ai_moments: "ai_moments" = true,
    moment_gap_s: "moment_gap_s" = 120,
    voice: "voice" = false,
});

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub schema: u32,
    pub units: Units,
    pub demo_mode: bool,
    pub ai: AiSettings,
    pub map: MapSettings,
    pub providers: ProviderSettings,
    pub trainer: TrainerSettings,
    pub ride: RideSettings,
    pub ride_coach: RideCoachSettings,
    pub diagnostics_opt_in: bool,
    pub theme: String,
}
json_struct!(Settings {
    schema: "schema" = SETTINGS_SCHEMA,
    units: "units" = Units::Metric,
    demo_mode: "demo_mode" = false,
    ai: "ai" = default_ai(),
    map: "map" = default_map(),
    providers: "providers" = default_providers(),
    trainer: "trainer" = default_trainer(),
    ride: "ride" = default_ride(),
    ride_coach: "ride_coach" = default_ride_coach(),
    diagnostics_opt_in: "diagnostics_opt_in" = false,
    theme: "theme" = "dark".to_string(),
});

fn from_empty<T: rl_json::FromJson>() -> T {
    T::from_json(&Value::empty_obj()).expect("defaults")
}
fn default_ai() -> AiSettings {
    from_empty()
}
fn default_map() -> MapSettings {
    from_empty()
}
fn default_providers() -> ProviderSettings {
    from_empty()
}
fn default_trainer() -> TrainerSettings {
    from_empty()
}
fn default_ride() -> RideSettings {
    from_empty()
}
fn default_ride_coach() -> RideCoachSettings {
    from_empty()
}

impl Default for Settings {
    fn default() -> Self {
        from_empty()
    }
}

fn check_url(u: &str, what: &str) -> Result<(), String> {
    rl_net::http::parse_url(u).map(|_| ()).map_err(|e| format!("{what}: {e}"))
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.ai.provider.as_str(), "offline" | "local" | "remote") {
            return Err("Unknown AI provider mode.".into());
        }
        check_url(&self.ai.base_url, "AI endpoint")?;
        if self.ai.model.trim().is_empty() || self.ai.model.len() > 120 {
            return Err("AI model name is required (max 120 characters).".into());
        }
        if self.ai.provider == "local" {
            let u = rl_net::http::parse_url(&self.ai.base_url).map_err(|e| e.to_string())?;
            if !rl_net::http::is_local_host(&u.host) {
                return Err("A 'local' AI endpoint must be on this computer (localhost). Use 'remote' for other hosts.".into());
            }
        }
        if !(1..=1000).contains(&self.ai.max_requests_per_day) || !(10..=900).contains(&self.ai.timeout_s) || !(200..=8000).contains(&self.ai.max_tokens) {
            return Err("AI limits are out of range.".into());
        }
        check_url(&self.map.style_url, "Map style")?;
        check_url(&self.providers.routing_url, "Routing service")?;
        check_url(&self.providers.elevation_url, "Elevation service")?;
        let t = &self.trainer;
        let finite = [t.difficulty_pct, t.lookahead_m, t.grade_min_pct, t.grade_max_pct, t.slew_pct_per_s].iter().all(|x| x.is_finite());
        if !finite
            || !(0.0..=100.0).contains(&t.difficulty_pct)
            || !(0.0..=30.0).contains(&t.lookahead_m)
            || !(-20.0..=0.0).contains(&t.grade_min_pct)
            || !(0.0..=25.0).contains(&t.grade_max_pct)
            || !(0.2..=10.0).contains(&t.slew_pct_per_s)
            || !(1000..=15_000).contains(&t.stale_ms)
            || !(500..=5000).contains(&t.target_interval_ms)
            || !(1000..=10_000).contains(&t.ack_timeout_ms)
        {
            return Err("Trainer settings are out of range.".into());
        }
        if !(60..=1800).contains(&self.ride_coach.moment_gap_s) {
            return Err("Ride coach comment gap must be 60–1800 seconds.".into());
        }
        if !matches!(self.theme.as_str(), "dark" | "light" | "system") {
            return Err("Unknown theme.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_json::{FromJson, ToJson};
    #[test]
    fn defaults_are_valid_and_private() {
        let s = Settings::default();
        assert!(s.validate().is_ok());
        assert_eq!(s.ai.provider, "offline", "no AI requests until the rider opts in");
        assert!(!s.ai.remote_enabled, "paid/remote integrations disabled by default");
        assert!(s.ride_coach.cues && s.ride_coach.ai_moments && !s.ride_coach.voice && s.ride.auto_pause);
        // Settings saved before the ride coach existed still load, with defaults.
        let mut old = s.to_json();
        if let Value::Obj(pairs) = &mut old {
            pairs.retain(|(k, _)| k != "ride_coach");
        }
        assert_eq!(Settings::from_json(&old).unwrap().ride_coach, s.ride_coach);
        let back = Settings::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);
        let mut bad = s.clone();
        bad.ai.provider = "local".into();
        bad.ai.base_url = "https://api.example.com/v1".into();
        assert!(bad.validate().is_err());
    }
}
