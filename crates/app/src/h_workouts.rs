//! Workout library and editor commands.

use crate::app::{pstr, R};
use crate::App;
use rl_domain::policy::{eligibility, level_for, rules_for};
use rl_domain::time::now_utc_ms;
use rl_domain::workout::{Category, Workout, WORKOUT_SCHEMA_VERSION};
use rl_json::{FromJson, ToJson, Value};

impl App {
    pub(crate) fn list_workouts(&mut self, p: &Value) -> R {
        let include_tests = p.bool_or("include_tests", false);
        let rules = self.profile().map(|pr| rules_for(level_for(&pr)));
        let list: Vec<Value> = self
            .library
            .iter()
            .filter(|w| include_tests || w.category != Category::TestFixture)
            .map(|w| {
                let mut v = w.summary_json();
                let el = rules.as_ref().map(|r| eligibility(w, r, 99, false));
                v.set("eligible", el.as_ref().map(|e| e.is_ok()).unwrap_or(true));
                v.set("eligibility_note", el.and_then(|e| e.err()));
                v
            })
            .collect();
        let cats: Vec<Value> = Category::all().iter().filter(|c| include_tests || **c != Category::TestFixture).map(|c| Value::obj([("id", c.to_json()), ("label", c.label().into())])).collect();
        Ok(Value::obj([("workouts", Value::Arr(list)), ("categories", Value::Arr(cats))]))
    }

    pub(crate) fn get_workout(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let w = self.workout(id).cloned().ok_or("Unknown workout.")?;
        let ftp = self.current_ftp().map(|f| f.watts);
        let tl: Vec<Value> = w
            .timeline()
            .iter()
            .map(|s| {
                let mut v = s.to_json();
                v.set("watts_start", s.target.watts_at(0.0, ftp));
                v.set("watts_end", s.target.watts_at(1.0, ftp));
                v.set("pct_start", s.target.pct_at(0.0, ftp));
                v.set("pct_end", s.target.pct_at(1.0, ftp));
                v
            })
            .collect();
        let mut v = w.to_json();
        v.set("summary", w.summary_json());
        v.set("timeline", Value::Arr(tl));
        v.set("ftp", ftp);
        Ok(v)
    }

    fn parse_custom(&self, p: &Value) -> Result<Workout, String> {
        let mut w = Workout::from_json(p.req("workout")?)?;
        w.builtin = false;
        w.schema = WORKOUT_SCHEMA_VERSION;
        w.policy_version = rl_domain::policy::POLICY_VERSION.into();
        w.name = w.name.trim().chars().filter(|c| !c.is_control()).collect();
        if w.family.is_empty() {
            w.family = "custom".into();
        }
        Ok(w)
    }

    pub(crate) fn validate_workout(&mut self, p: &Value) -> R {
        let w = self.parse_custom(p)?;
        let issues = w.validate();
        Ok(Value::obj([("issues", issues.to_json()), ("duration_s", w.total_s().into()), ("intensity", w.intensity().to_json())]))
    }

    pub(crate) fn save_workout(&mut self, p: &Value) -> R {
        let mut w = self.parse_custom(p)?;
        if w.category == Category::TestFixture {
            return Err("Choose a training category.".into());
        }
        match self.workout(&w.id) {
            Some(existing) if existing.builtin => return Err("Built-in workouts can't be edited; duplicate it first.".into()),
            Some(existing) => {
                w.version = existing.version + 1;
                w.created_utc = existing.created_utc;
            }
            None => {
                w.id = rl_domain::ids::new_uuid();
                w.version = 1;
                w.created_utc = now_utc_ms();
            }
        }
        w.updated_utc = now_utc_ms();
        let issues = w.validate();
        if !issues.is_empty() {
            return Err(format!("{} ({})", issues[0].message, issues[0].path));
        }
        self.store.save_workout(&w)?;
        self.reload_library();
        Ok(Value::obj([("id", w.id.clone().into()), ("version", w.version.into())]))
    }

    pub(crate) fn duplicate_workout(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let mut w = self.workout(id).cloned().ok_or("Unknown workout.")?;
        w.id = rl_domain::ids::new_uuid();
        w.builtin = false;
        w.version = 1;
        w.name = format!("{} (copy)", w.name).chars().take(80).collect();
        if w.category == Category::TestFixture {
            w.category = Category::Endurance;
        }
        w.created_utc = now_utc_ms();
        w.updated_utc = w.created_utc;
        self.store.save_workout(&w)?;
        self.reload_library();
        Ok(Value::obj([("id", w.id.into())]))
    }

    pub(crate) fn delete_workout(&mut self, p: &Value) -> R {
        let id = pstr(p, "id", 64)?;
        let w = self.workout(id).ok_or("Unknown workout.")?;
        if w.builtin {
            return Err("Built-in workouts can't be deleted.".into());
        }
        if let Some(plan) = self.store.current_plan()? {
            if plan.sessions.iter().any(|s| s.workout_id == id && s.date >= self.today()) {
                return Err("This workout is scheduled in your plan. Replace it there first.".into());
            }
        }
        self.store.delete_workout(id)?;
        self.reload_library();
        Ok(Value::Null)
    }
}
