//! Session acceptance scenarios against the deterministic simulator.
//! These exercise A04/A05/A08/A09-style behaviour in software only; they are
//! not hardware evidence.

use rl_device::controller::ControlState;
use rl_device::manager::DeviceManager;
use rl_device::simulator::{SimulatorAdapter, TrainerMode};
use rl_domain::geo::LatLon;
use rl_domain::library::test_fixtures;
use rl_domain::route::{process, synthetic, ElevationSource, ProfileConfig, RouteInput};
use rl_session::coordinator::*;
use rl_session::record::{flags, MemorySink};
use rl_session::route_engine::GradeLimits;
use std::sync::Arc;

struct Rig {
    dm: DeviceManager,
    t: u64,
}

impl Rig {
    fn new() -> Rig {
        let mut dm = DeviceManager::new(vec![Box::new(SimulatorAdapter::new())]);
        let mut t = 0;
        dm.tick(t);
        dm.start_scan(None, t).unwrap();
        for _ in 0..20 {
            t += 50;
            dm.tick(t);
        }
        for k in ["simulator:sim-trainer", "simulator:sim-hrm"] {
            dm.connect(k, t).unwrap();
        }
        let mut r = Rig { dm, t };
        r.idle(3000);
        r
    }
    fn sim(&mut self) -> &mut SimulatorAdapter {
        self.dm.adapters[0].as_any_mut().downcast_mut::<SimulatorAdapter>().unwrap()
    }
    fn idle(&mut self, ms: u64) {
        let end = self.t + ms;
        while self.t < end {
            self.t += 100;
            self.dm.tick(self.t);
        }
    }
    fn ride(&mut self, s: &mut Session, ms: u64) {
        let end = self.t + ms;
        while self.t < end {
            self.t += 100;
            self.dm.tick(self.t);
            s.tick(self.t, 1_790_000_000_000 + self.t as i64, &mut self.dm);
        }
    }
    fn mode(&mut self) -> TrainerMode {
        self.sim().trainer().mode.clone()
    }
}

fn spec(mode: RideMode) -> PrepareSpec {
    PrepareSpec {
        mode,
        workout: None,
        route: None,
        ftp_w: Some(250.0),
        rpe_mode: false,
        difficulty_pct: 100.0,
        lookahead_m: 0.0,
        system_mass_kg: 84.0,
        grade_limits: GradeLimits::default(),
        manual_level: 20.0,
        demo: true,
        plan_session_id: None,
        tz_name: "UTC".into(),
        tz_offset_min: 0,
    }
}

fn erg_spec() -> PrepareSpec {
    let mut s = spec(RideMode::Erg);
    s.workout = Some(test_fixtures().into_iter().find(|w| w.id == "test-erg-short").unwrap());
    s
}

#[test]
fn a04_erg_workout_runs_pauses_and_finishes() {
    let mut r = Rig::new();
    let sp = erg_spec();
    let pf = preflight(&sp, &r.dm, r.t);
    assert!(pf.iter().all(|p| p.status != "block"), "{pf:?}");
    let mut s = Session::new("s1".into(), sp, 0, Some(Box::new(MemorySink::default())));
    s.start(r.t, 0, &mut r.dm).unwrap();
    assert_eq!(s.state, SessionState::Starting);
    r.ride(&mut s, 30_000);
    assert_eq!(s.state, SessionState::Running);
    assert_eq!(r.dm.controller.state, ControlState::Controlled);
    assert_eq!(r.mode(), TrainerMode::Erg(100), "after ramp-in the first step target is reached");
    r.ride(&mut s, 40_000);
    assert_eq!(r.mode(), TrainerMode::Erg(150));
    // Pause freezes the timer and eases the load.
    let active = s.active_ms;
    s.pause(r.t, 0, &mut r.dm);
    r.ride(&mut s, 10_000);
    assert_eq!(s.active_ms, active);
    assert_eq!(r.mode(), TrainerMode::Erg(0), "low-load target during pause");
    s.resume(r.t, 0, &mut r.dm);
    r.ride(&mut s, 130_000);
    assert!(s.workout.as_ref().unwrap().complete);
    s.stop(r.t, 0, &mut r.dm, true);
    r.ride(&mut s, 3000);
    assert_eq!(s.state, SessionState::Finished);
    assert_eq!(s.stop_confirmed, Some(true));
    let sum = s.summary.as_ref().unwrap();
    assert!(sum.timer_s >= 180.0 && sum.timer_s <= 201.0, "{}", sum.timer_s);
    assert!(sum.elapsed_s > sum.timer_s, "pause is in elapsed time only");
    assert!(s.laps.len() >= 3, "{}", s.laps.len());
    assert!(sum.power.coverage > 0.9);
    assert!(s.samples.iter().all(|x| x.flags & flags::DEMO != 0), "demo rides are labelled");
    // Targets never exceeded the device limit.
    assert!(s.samples.iter().filter_map(|x| x.target_w).all(|w| w <= 2000.0));
}

#[test]
fn a05_free_ride_sends_signed_grades() {
    let mut r = Rig::new();
    let pts = synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(300.0, 0.0), (400.0, 5.0), (200.0, 0.0), (400.0, -3.0), (200.0, 0.0)], false);
    let segs = vec![pts];
    let src = ElevationSource { kind: "synthetic".into(), dataset: String::new(), resolution_m: 0.0, fetched_utc: 0, attribution: String::new() };
    let prof = Arc::new(process(&RouteInput { segments: &segs, source: src, corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap());
    let mut sp = spec(RideMode::FreeRide);
    sp.route = Some(("r".into(), "A05".into(), prof));
    let mut s = Session::new("s2".into(), sp, 0, None);
    s.start(r.t, 0, &mut r.dm).unwrap();
    let mut seen_pos = false;
    let mut seen_neg = false;
    for _ in 0..1200 {
        r.ride(&mut s, 1000);
        if let TrainerMode::Simulation(p) = r.mode() {
            let road = s.route.as_ref().unwrap().road_grade.unwrap_or(0.0);
            if p.grade_percent > 4.5 {
                seen_pos = true;
                assert!(road > 4.0, "trainer climbs only where the road climbs (road {road})");
            }
            if p.grade_percent < -2.5 {
                seen_neg = true;
                assert!(road < -2.0);
            }
        }
        if s.route.as_ref().unwrap().finished {
            break;
        }
    }
    assert!(seen_pos && seen_neg);
    assert!(s.route.as_ref().unwrap().finished);
    r.ride(&mut s, 5000);
    if let TrainerMode::Simulation(p) = r.mode() {
        assert!(p.grade_percent.abs() < 0.3, "eased to flat at the finish");
    }
    let last = s.samples.last().unwrap();
    assert!(last.grade.is_some() && last.lat.is_some() && last.distance.unwrap() > 1490.0);
}

#[test]
fn a09_disconnect_under_load_requires_controlled_resumption() {
    let mut r = Rig::new();
    let mut sp = erg_spec();
    sp.workout = Some(test_fixtures().into_iter().find(|w| w.id == "test-erg-pause").unwrap());
    let mut s = Session::new("s3".into(), sp, 0, None);
    s.start(r.t, 0, &mut r.dm).unwrap();
    r.ride(&mut s, 20_000);
    r.sim().set_fault("sim-trainer", "offline", true);
    r.ride(&mut s, 5000);
    assert!(s.control_interrupted);
    let gap: Vec<_> = s.samples.iter().rev().take(3).collect();
    assert!(gap.iter().all(|x| x.power.is_none() && x.flags & flags::CONTROL_LOST != 0), "stale values are not recorded as data");
    r.sim().set_fault("sim-trainer", "offline", false);
    r.ride(&mut s, 25_000);
    assert!(s.needs_resume_control, "reconnected trainer waits for the rider");
    assert_ne!(r.dm.controller.state, ControlState::Controlled);
    let sent = r.dm.controller.commands_sent;
    r.ride(&mut s, 3000);
    assert_eq!(r.dm.controller.commands_sent, sent, "nothing sent before the rider resumes control");
    s.resume_control(r.t, 0, &mut r.dm).unwrap();
    r.ride(&mut s, 2000);
    assert!(!s.needs_resume_control);
    // Ramp-in: shortly after regaining control the target is well below the step target.
    match r.mode() {
        TrainerMode::Erg(w) => assert!(w < 120, "{w}"),
        m => panic!("{m:?}"),
    }
}

#[test]
fn low_cadence_eases_load_and_recovers_with_rider_confirmation() {
    let mut r = Rig::new();
    r.dm.connect("simulator:sim-cadence", r.t).unwrap();
    r.idle(2000);
    let mut sp = erg_spec();
    sp.workout = Some(test_fixtures().into_iter().find(|w| w.id == "test-erg-pause").unwrap());
    let mut s = Session::new("s4".into(), sp, 0, None);
    s.start(r.t, 0, &mut r.dm).unwrap();
    r.ride(&mut s, 15_000);
    r.sim().rider.cadence_rpm = 25.0;
    r.ride(&mut s, 9000);
    assert!(s.low_cadence_active());
    match r.mode() {
        TrainerMode::Erg(w) => assert!(w <= 70, "eased: {w}"),
        m => panic!("{m:?}"),
    }
    r.sim().rider.cadence_rpm = 90.0;
    r.ride(&mut s, 6000);
    assert!(s.low_cadence_active(), "no automatic re-escalation without rider confirmation");
    s.acknowledge_low_cadence(r.t, 0);
    r.ride(&mut s, 5000);
    assert!(!s.low_cadence_active());
}

#[test]
fn a08_mode_switch_leaves_one_controller_and_drops_stale_commands() {
    let mut r = Rig::new();
    let mut s = Session::new("s5".into(), erg_spec(), 0, None);
    s.start(r.t, 0, &mut r.dm).unwrap();
    r.ride(&mut s, 15_000);
    let g1 = r.dm.controller.generation();
    s.switch_mode(RideMode::ReadOnly, r.t, 0, &mut r.dm).unwrap();
    assert!(r.dm.controller.generation() > g1);
    assert_eq!(s.owner, Owner::None);
    let n = r.dm.controller.commands_sent;
    r.ride(&mut s, 10_000);
    assert!(r.dm.controller.commands_sent <= n + 2, "only the release (low-load + pause) is sent");
    assert!(s.events.iter().any(|e| e.kind == "mode_change"));
    s.switch_mode(RideMode::Erg, r.t, 0, &mut r.dm).unwrap();
    assert_eq!(s.owner, Owner::Workout);
    assert!(s.switch_mode(RideMode::FreeRide, r.t, 0, &mut r.dm).is_err(), "no route in this ride");
}

#[test]
fn read_only_ride_never_commands_the_trainer() {
    let mut r = Rig::new();
    let mut s = Session::new("s6".into(), spec(RideMode::ReadOnly), 0, None);
    s.start(r.t, 0, &mut r.dm).unwrap();
    assert_eq!(s.state, SessionState::Running);
    r.ride(&mut s, 10_000);
    assert_eq!(r.dm.controller.commands_sent, 0);
    s.stop(r.t, 0, &mut r.dm, true);
    assert_eq!(s.state, SessionState::Finished);
    assert!(s.samples.len() >= 9);
}

#[test]
fn denied_control_returns_to_prepared() {
    let mut r = Rig::new();
    r.sim().set_fault("sim-trainer", "deny_control", true);
    let mut s = Session::new("s7".into(), erg_spec(), 0, None);
    s.start(r.t, 0, &mut r.dm).unwrap();
    r.ride(&mut s, 3000);
    assert_eq!(s.state, SessionState::Prepared);
    assert!(s.notices.iter().any(|n| n.level == "error"));
}

#[test]
fn erg_without_ftp_is_blocked_in_preflight() {
    let r = Rig::new();
    let mut sp = spec(RideMode::Erg);
    sp.ftp_w = None;
    sp.workout = Some(rl_domain::library::builtin_workouts().into_iter().find(|w| w.id == "sweet-spot-3x8").unwrap());
    let pf = preflight(&sp, &r.dm, r.t);
    assert!(pf.iter().any(|p| p.key == "workout" && p.status == "block"));
    sp.rpe_mode = true;
    let pf = preflight(&sp, &r.dm, r.t);
    assert!(!pf.iter().any(|p| p.key == "workout" && p.status == "block"));
}

#[test]
fn a16_long_session_has_no_drift_or_duplication() {
    let mut r = Rig::new();
    let mut s = Session::new("s8".into(), spec(RideMode::ReadOnly), 0, None);
    s.start(r.t, 0, &mut r.dm).unwrap();
    // Two hours at an irregular tick (97 ms) to expose accumulation errors.
    let end = r.t + 2 * 3600 * 1000;
    while r.t < end {
        r.t += 97;
        r.dm.tick(r.t);
        s.tick(r.t, r.t as i64, &mut r.dm);
    }
    let n = s.samples.len() as i64;
    assert!((n - 7200).abs() <= 1, "{n}");
    for w in s.samples.windows(2) {
        assert_eq!(w[1].active_s, w[0].active_s + 1, "no duplicated or skipped seconds");
    }
    assert!(!r.dm.controller.has_pending());
}

#[test]
fn ride_cues_preview_announce_and_suggest_without_touching_control() {
    use rl_domain::library::builtin_workouts;
    use rl_session::ride_coach::CueAction;
    let mut r = Rig::new();
    let mut sp = erg_spec();
    sp.ftp_w = Some(400.0);
    sp.workout = Some(builtin_workouts().into_iter().find(|w| w.id == "threshold-4x5").unwrap());
    let mut s = Session::new("c1".into(), sp, 0, Some(Box::new(MemorySink::default())));
    s.start(r.t, 0, &mut r.dm).unwrap();
    // The simulated trainer can only hold 300 W: threshold at 400 W leaves the rider under target.
    r.sim().set_fault("sim-trainer", "low_power_limit", true);
    r.ride(&mut s, 800_000);
    let kinds: Vec<&str> = s.coach.feed.iter().map(|f| f.kind.as_str()).collect();
    let texts: Vec<&str> = s.coach.feed.iter().map(|f| f.text.as_str()).collect();
    assert!(texts.iter().any(|t| t.starts_with("Warm-up: 12 min")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("In 15 s:") && t.contains("5 min at 400 W")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("5 min at 400 W") && t.contains("Hard and steady")), "{texts:?}");
    let under = s.coach.feed.iter().find(|f| f.kind == "under_target").expect("under-target cue");
    assert_eq!(under.action, Some(CueAction::Intensity(-5)));
    assert!(kinds.iter().filter(|k| **k == "under_target").count() == 1, "rate limited: {kinds:?}");
    assert_eq!(s.workout.as_ref().unwrap().adjust_pct, 0, "a suggestion is never applied by itself");
    let m = s.coach.take_moment().expect("hard interval start is a moment");
    assert_eq!(m.kind, "interval_start");
    // Halfway and the last hard interval.
    r.ride(&mut s, 1_800_000);
    let texts: Vec<String> = s.coach.feed.iter().map(|f| f.text.clone()).collect();
    assert!(texts.iter().any(|t| t.starts_with("Halfway")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Last hard one!")), "{texts:?}");
    // Cues are recorded with the ride.
    assert!(s.events.iter().any(|e| e.kind == "coach" && e.detail.contains("under_target")));
    let snap = s.coach_snapshot(r.t, &r.dm);
    assert!(snap.get("workout").and_then(|w| w.get("step")).is_some());
    assert!(snap.get("route").map(|v| v.is_null()).unwrap_or(true), "no location in the snapshot");
}

#[test]
fn ride_cues_announce_climbs_and_can_be_turned_off() {
    let pts = synthetic(LatLon { lat: 0.0, lon: 0.0 }, &[(600.0, 0.0), (1000.0, 5.0), (400.0, 0.0)], false);
    let segs = vec![pts];
    let src = ElevationSource { kind: "synthetic".into(), dataset: String::new(), resolution_m: 0.0, fetched_utc: 0, attribution: String::new() };
    let prof = Arc::new(process(&RouteInput { segments: &segs, source: src, corrections: &[], flat_fallback: false }, &ProfileConfig::default()).unwrap());
    for cues_on in [true, false] {
        let mut r = Rig::new();
        let mut sp = spec(RideMode::FreeRide);
        sp.route = Some(("r".into(), "Climb".into(), prof.clone()));
        let mut s = Session::new("c2".into(), sp, 0, None);
        s.coach_cues = cues_on;
        s.start(r.t, 0, &mut r.dm).unwrap();
        for _ in 0..900 {
            r.ride(&mut s, 1000);
            if s.route.as_ref().unwrap().finished {
                break;
            }
        }
        let texts: Vec<&str> = s.coach.feed.iter().map(|f| f.text.as_str()).collect();
        if !cues_on {
            assert!(texts.is_empty(), "{texts:?}");
            continue;
        }
        let climb = texts.iter().find(|t| t.starts_with("Climb in")).unwrap_or_else(|| panic!("{texts:?}"));
        // e.g. "Climb in 600 m: 950 m at 5.0% average." (smoothing trims the edges slightly)
        let avg: f64 = climb.split(" at ").nth(1).and_then(|x| x.split('%').next()).and_then(|x| x.parse().ok()).unwrap();
        assert!((4.5..=5.5).contains(&avg), "{climb}");
        assert_eq!(texts.iter().filter(|t| t.starts_with("Climb in")).count(), 1, "announced once: {texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("Top of the climb")), "{texts:?}");
        assert_eq!(s.coach.take_moment().map(|m| m.kind), Some("climb_ahead".into()), "a 1 km climb is a moment");
    }
}
