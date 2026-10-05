//! Onboarding, rider profile and FTP history.

use crate::app::{pstr, R};
use crate::App;
use rl_domain::rider::{Confidence, FtpEntry, FtpMethod, RiderProfile};
use rl_domain::time::{now_utc_ms, Date};
use rl_json::{FromJson, ToJson, Value};

impl App {
    pub(crate) fn save_onboarding_draft(&mut self, p: &Value) -> R {
        let d = p.req("draft")?;
        if d.to_string_compact().len() > 64 * 1024 {
            return Err("Draft too large.".into());
        }
        self.store.write_value("onboarding_draft.json", d)?;
        Ok(Value::Null)
    }

    fn parse_ftp(&self, v: &Value) -> Result<FtpEntry, String> {
        let e = FtpEntry {
            id: rl_domain::ids::new_uuid(),
            watts: v.req_f64("watts")?.round(),
            effective: match v.opt_str("effective")? {
                Some(s) => Date::parse(s).ok_or("Invalid FTP date.")?,
                None => self.today(),
            },
            method: FtpMethod::parse(v.str_or("method", "known")).ok_or("Unknown FTP method.")?,
            confidence: Confidence::parse(v.str_or("confidence", "medium")).ok_or("Unknown confidence.")?,
            provisional: v.bool_or("provisional", false),
            note: v.str_or("note", "").chars().take(300).collect(),
            created_utc: now_utc_ms(),
        };
        e.validate()?;
        if e.effective > self.today() {
            return Err("The FTP date cannot be in the future.".into());
        }
        Ok(e)
    }

    pub(crate) fn complete_onboarding(&mut self, p: &Value) -> R {
        let mut profile = RiderProfile::from_json(p.req("profile")?)?;
        let now = now_utc_ms();
        profile.created_utc = if profile.created_utc == 0 { now } else { profile.created_utc };
        profile.updated_utc = now;
        profile.ai_consent.updated_utc = now;
        profile.validate()?;
        self.store.save_profile(&profile)?;
        if let Some(f) = p.get("ftp").filter(|f| !f.is_null()) {
            let e = self.parse_ftp(f)?;
            let mut h = self.ftp_history();
            h.push(e);
            self.store.save_ftp_history(&h)?;
        }
        self.settings.units = profile.units;
        self.settings.trainer.difficulty_pct = profile.trainer_difficulty_pct;
        self.store.write_value("settings.json", &self.settings.to_json())?;
        self.store.remove("onboarding_draft.json")?;
        Ok(profile.to_json())
    }

    /// "Try a demo": simulated devices, a clearly labelled demo profile and a
    /// demo FTP value (marked provisional; not derived from the rider).
    pub(crate) fn start_demo(&mut self, _p: &Value) -> R {
        if self.profile().is_none() {
            let mut prof = RiderProfile::demo();
            prof.created_utc = now_utc_ms();
            prof.updated_utc = prof.created_utc;
            self.store.save_profile(&prof)?;
            if self.ftp_history().is_empty() {
                let e = FtpEntry {
                    id: rl_domain::ids::new_uuid(),
                    watts: 200.0,
                    effective: self.today(),
                    method: FtpMethod::Manual,
                    confidence: Confidence::Low,
                    provisional: true,
                    note: "Demo value for the simulated rider. Replace it with your own FTP, or delete it.".into(),
                    created_utc: now_utc_ms(),
                };
                self.store.save_ftp_history(&[e])?;
            }
        }
        self.enable_demo(true);
        Ok(Value::Null)
    }

    pub(crate) fn get_profile(&mut self, _p: &Value) -> R {
        let profile = self.profile();
        let level = profile.as_ref().map(|p| rl_domain::policy::level_for(p));
        let mut h = self.ftp_history();
        h.sort_by(|a, b| b.effective.cmp(&a.effective));
        Ok(Value::obj([
            ("profile", profile.map(|p| p.to_json()).unwrap_or(Value::Null)),
            ("level", level.map(|l| l.to_json()).unwrap_or(Value::Null)),
            ("level_label", level.map(|l| l.label()).into()),
            ("ftp", self.current_ftp().map(|f| f.to_json()).unwrap_or(Value::Null)),
            ("ftp_history", h.to_json()),
        ]))
    }

    pub(crate) fn save_profile(&mut self, p: &Value) -> R {
        let mut profile = RiderProfile::from_json(p.req("profile")?)?;
        let old = self.profile();
        profile.created_utc = old.as_ref().map(|o| o.created_utc).unwrap_or_else(now_utc_ms);
        profile.updated_utc = now_utc_ms();
        if old.as_ref().map(|o| o.ai_consent != profile.ai_consent).unwrap_or(true) {
            profile.ai_consent.updated_utc = now_utc_ms();
        }
        self.store.save_profile(&profile)?;
        self.settings.units = profile.units;
        self.store.write_value("settings.json", &self.settings.to_json())?;
        Ok(profile.to_json())
    }

    pub(crate) fn add_ftp(&mut self, p: &Value) -> R {
        let e = self.parse_ftp(p)?;
        let mut h = self.ftp_history();
        h.push(e.clone());
        self.store.save_ftp_history(&h)?;
        Ok(e.to_json())
    }

    pub(crate) fn delete_ftp(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let mut h = self.ftp_history();
        let n = h.len();
        h.retain(|e| e.id != id);
        if h.len() == n {
            return Err("Unknown FTP entry.".into());
        }
        self.store.save_ftp_history(&h)?;
        Ok(Value::Null)
    }

    /// Provisional FTP estimate from a completed optional assessment; the
    /// rider must accept it explicitly.
    pub(crate) fn accept_ftp_estimate(&mut self, p: &Value) -> R {
        let act = pstr(p, "activity_id", 64)?;
        let a = self.store.load_activity(act)?.ok_or("Unknown activity.")?;
        if a.meta.demo {
            return Err("Demo rides cannot set your FTP.".into());
        }
        let est = crate::h_history::ftp_estimate(&a).ok_or("This ride has no assessment result.")?;
        let e = FtpEntry {
            id: rl_domain::ids::new_uuid(),
            watts: est.0,
            effective: Date::from_utc_ms(a.meta.start_utc, a.meta.tz_offset_min),
            method: est.1,
            confidence: Confidence::Medium,
            provisional: true,
            note: format!("Provisional estimate from '{}'.", a.meta.title),
            created_utc: now_utc_ms(),
        };
        e.validate()?;
        let mut h = self.ftp_history();
        h.push(e.clone());
        self.store.save_ftp_history(&h)?;
        Ok(e.to_json())
    }
}
