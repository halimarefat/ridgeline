//! End-to-end command-level tests of the application service with the
//! simulator (no hardware, no network).

use rl_app::{App, Config, NullPlatform};
use rl_json::{parse, Value};
use rl_net::http::StdHttpClient;
use std::path::PathBuf;
use std::sync::Arc;

fn tmpdir() -> PathBuf {
    std::env::temp_dir().join(format!("rl-app-{}", rl_domain::ids::new_uuid()))
}

fn mk(dir: &PathBuf) -> App {
    let cfg = Config { data_dir: dir.join("data"), export_dir: dir.join("exports"), platform_name: "test".into(), app_version: "test".into() };
    App::new(cfg, vec![], Arc::new(StdHttpClient), Arc::new(NullPlatform)).unwrap()
}

fn call(app: &mut App, m: &str, p: &str) -> Value {
    match app.rpc(m, p) {
        Ok(v) => v,
        Err(e) => panic!("{m} failed: {e}"),
    }
}

fn run(app: &mut App, ms: u64) {
    let mut t = 0;
    while t < ms {
        app.advance_clock(100);
        app.tick();
        t += 100;
    }
}

#[test]
fn demo_onboarding_plan_ride_export_recovery() {
    let dir = tmpdir();
    let mut app = mk(&dir);
    let b = call(&mut app, "getBootstrap", r#"{"tz_name":"America/St_Johns","tz_offset_min":-150}"#);
    assert_eq!(b.get("onboarded").unwrap().as_bool(), Some(false));
    call(&mut app, "startDemo", "{}");
    run(&mut app, 4000);
    let st = call(&mut app, "getState", "{}");
    let devs = st.get("devices").unwrap().get("devices").unwrap().as_arr().unwrap();
    assert!(devs.iter().filter(|d| d.get("state").unwrap().as_str() == Some("ready")).count() >= 3, "{}", st.get("devices").unwrap());

    // Offline plan → proposal → accept → undo → accept again.
    let r = call(&mut app, "proposePlan", r#"{"weeks":4,"use_ai":false}"#);
    let pid = r.get("proposal_id").unwrap().as_str().unwrap().to_string();
    let prop = call(&mut app, "getProposal", &format!(r#"{{"id":"{pid}"}}"#));
    assert_eq!(prop.get("source").unwrap().as_str(), Some("offline"));
    assert!(call(&mut app, "getPlan", "{}").get("plan").unwrap().is_null(), "nothing changes until accepted");
    call(&mut app, "acceptProposal", &format!(r#"{{"id":"{pid}"}}"#));
    let plan = call(&mut app, "getPlan", "{}");
    let sessions = plan.get("plan").unwrap().get("sessions").unwrap().as_arr().unwrap().clone();
    assert!(!sessions.is_empty());
    assert_eq!(plan.get("can_undo").unwrap().as_bool(), Some(true));
    call(&mut app, "undoPlan", "{}");
    assert!(call(&mut app, "getPlan", "{}").get("plan").unwrap().is_null());

    // ERG ride with the short test workout.
    let pf = call(&mut app, "preflight", r#"{"mode":"erg","workout_id":"test-erg-short"}"#);
    assert_eq!(pf.get("ok").unwrap().as_bool(), Some(true), "{pf}");
    call(&mut app, "startSession", r#"{"mode":"erg","workout_id":"test-erg-short"}"#);
    run(&mut app, 60_000);
    let st = call(&mut app, "getState", "{}");
    let s = st.get("session").unwrap();
    assert_eq!(s.get("state").unwrap().as_str(), Some("running"));
    assert!(s.get("power").unwrap().get("value").unwrap().as_f64().is_some());
    call(&mut app, "pauseSession", "{}");
    run(&mut app, 5000);
    call(&mut app, "resumeSession", "{}");
    run(&mut app, 130_000);
    call(&mut app, "stopSession", r#"{"save":true}"#);
    run(&mut app, 3000);
    let st = call(&mut app, "getState", "{}");
    assert_eq!(st.get("session").unwrap().get("state").unwrap().as_str(), Some("finished"));
    let acts = call(&mut app, "listActivities", "{}");
    let a = &acts.as_arr().unwrap()[0];
    assert_eq!(a.get("meta").unwrap().get("status").unwrap().as_str(), Some("complete"));
    assert_eq!(a.get("meta").unwrap().get("demo").unwrap().as_bool(), Some(true));
    let aid = a.get("meta").unwrap().get("id").unwrap().as_str().unwrap().to_string();
    call(&mut app, "closeSession", "{}");
    for fmt in ["fit", "csv"] {
        let r = call(&mut app, "exportActivity", &format!(r#"{{"id":"{aid}","format":"{fmt}"}}"#));
        let path = r.get("path").unwrap().as_str().unwrap();
        let bytes = std::fs::read(path).unwrap();
        if fmt == "fit" {
            assert_eq!(rl_storage::fit::crc16(0, &bytes), 0);
        } else {
            assert!(String::from_utf8(bytes).unwrap().starts_with("timestamp_utc,"));
        }
    }
    // Feedback on a demo ride never adapts the plan.
    let fb = call(&mut app, "submitFeedback", &format!(r#"{{"activity_id":"{aid}","rpe":9,"fatigue":5,"enjoyment":2}}"#));
    assert!(fb.get("proposal_id").unwrap().is_null());

    // Crash during a ride: process dies without stopping.
    call(&mut app, "startSession", r#"{"mode":"read_only"}"#);
    run(&mut app, 12_000);
    drop(app);
    let mut app = mk(&dir);
    let b = call(&mut app, "getBootstrap", "{}");
    let rec = b.get("recovery").unwrap().as_arr().unwrap();
    assert_eq!(rec.len(), 1, "interrupted ride offered for recovery");
    // No trainer is started automatically after restart.
    assert_eq!(app.dm.controller.commands_sent, 0);
    let id = rec[0].get("id").unwrap().as_str().unwrap().to_string();
    let r = call(&mut app, "recoverActivity", &format!(r#"{{"id":"{id}"}}"#));
    let timer = r.get("summary").unwrap().get("timer_s").unwrap().as_f64().unwrap();
    assert!(timer >= 9.0, "at most a few seconds lost: {timer}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn routes_free_ride_offline_and_coach() {
    let dir = tmpdir();
    let mut app = mk(&dir);
    call(&mut app, "startDemo", "{}");
    run(&mut app, 4000);
    let routes = call(&mut app, "listRoutes", "{}");
    assert!(routes.as_arr().unwrap().len() >= 2);
    // GPX import without elevation: simulation blocked until fallback/elevation.
    let gpx = r#"<gpx><trk><name>Harbour <b>loop</b></name><trkseg><trkpt lat="47.56" lon="-52.71"/><trkpt lat="47.561" lon="-52.71"/><trkpt lat="47.562" lon="-52.709"/></trkseg></trk></gpx>"#;
    let r = call(&mut app, "importGpx", &Value::obj([("text", gpx.into())]).to_string_compact());
    assert_eq!(r.get("needs_elevation").unwrap().as_bool(), Some(true));
    let rid = r.get("id").unwrap().as_str().unwrap().to_string();
    let pf = call(&mut app, "preflight", &format!(r#"{{"mode":"free_ride","route_id":"{rid}"}}"#));
    assert_eq!(pf.get("ok").unwrap().as_bool(), Some(false));
    call(&mut app, "setFlatFallback", &format!(r#"{{"id":"{rid}","on":true}}"#));
    let pf = call(&mut app, "preflight", &format!(r#"{{"mode":"free_ride","route_id":"{rid}"}}"#));
    assert_eq!(pf.get("ok").unwrap().as_bool(), Some(true), "{pf}");
    // A10: free ride on the bundled demo route with no network at all.
    call(&mut app, "startSession", r#"{"mode":"free_ride","route_id":"demo-null-island-loop"}"#);
    run(&mut app, 120_000);
    let st = call(&mut app, "getState", "{}");
    let route = st.get("session").unwrap().get("route").unwrap();
    assert!(route.get("s").unwrap().as_f64().unwrap() > 300.0);
    assert!(route.get("commanded_grade").unwrap().as_f64().is_some());
    call(&mut app, "stopSession", r#"{"save":true}"#);
    run(&mut app, 3000);
    call(&mut app, "closeSession", "{}");
    // Offline coach answers without AI.
    let r = call(&mut app, "coachChat", r#"{"message":"what should I ride next?"}"#);
    assert_eq!(r.get("source").unwrap().as_str(), Some("offline"));
    let r = call(&mut app, "coachChat", r#"{"message":"I felt dizzy and had chest pain"}"#);
    assert_eq!(r.get("safety").unwrap().as_bool(), Some(true));
    // Bad input is rejected, not panicking.
    assert!(app.rpc("importGpx", r#"{"text":"<!DOCTYPE x [<!ENTITY a SYSTEM 'file:///etc/passwd'>]><gpx/>"}"#).is_err());
    assert!(app.rpc("getRoute", r#"{"id":"../../secret"}"#).is_err());
    assert!(app.rpc("nope", "{}").is_err());
    assert!(app.rpc("getState", "[1,2]").is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn single_instance_lock() {
    let dir = tmpdir();
    let cfg = Config { data_dir: dir.join("data"), export_dir: dir.join("exports"), platform_name: "test".into(), app_version: "test".into() };
    let r1 = rl_app::Runtime::start(cfg.clone(), vec![], Arc::new(StdHttpClient), Arc::new(NullPlatform)).unwrap();
    let r2 = rl_app::Runtime::start(cfg, vec![], Arc::new(StdHttpClient), Arc::new(NullPlatform));
    assert!(r2.err().unwrap().contains("already running"));
    let out = parse(&r1.rpc("getBootstrap", "{}")).unwrap();
    assert_eq!(out.get("ok").unwrap().as_bool(), Some(true));
    drop(r1);
    std::fs::remove_dir_all(&dir).ok();
}

fn feed(app: &mut App) -> Vec<Value> {
    let st = call(app, "getState", "{}");
    st.get("session").unwrap().get("coach").unwrap().as_arr().unwrap().clone()
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

#[test]
fn ride_coach_offline_cues_prompts_and_accepted_suggestion() {
    let dir = tmpdir();
    let mut app = mk(&dir);
    call(&mut app, "startDemo", "{}");
    run(&mut app, 4000);
    call(&mut app, "startSession", r#"{"mode":"erg","workout_id":"test-erg-short"}"#);
    run(&mut app, 20_000);
    let f = feed(&mut app);
    assert!(f.iter().any(|x| s(x, "from") == "cue" && s(x, "text").starts_with("Warm-up: 60 s at 100 W")), "{f:?}");
    // Quick prompt with no AI configured: the offline coach answers at once.
    let r = call(&mut app, "rideCoach", r#"{"trigger":"too_hard"}"#);
    assert!(r.get("job_id").unwrap().is_null());
    let f = feed(&mut app);
    let n = f.len();
    assert_eq!(s(&f[n - 2], "from"), "rider");
    assert_eq!(s(&f[n - 1], "from"), "coach");
    let action = f[n - 1].get("action").unwrap();
    assert_eq!((s(action, "kind"), action.get("delta").unwrap().as_i64()), ("intensity".into(), Some(-5)));
    // Nothing changed until the rider presses the suggestion.
    let wk = call(&mut app, "getState", "{}").get("session").unwrap().get("workout").unwrap().clone();
    assert_eq!(wk.get("adjust_pct").unwrap().as_i64(), Some(0));
    call(&mut app, "adjustIntensity", r#"{"delta":-5,"via":"coach"}"#);
    let f = feed(&mut app);
    assert!(s(f.last().unwrap(), "text").contains("Intensity is now -5%"), "{f:?}");
    // Symptoms: fixed safety copy with a stop suggestion, never an AI call.
    call(&mut app, "rideCoach", r#"{"trigger":"message","text":"I have chest pain"}"#);
    let f = feed(&mut app);
    let last = f.last().unwrap();
    assert_eq!(s(last, "from"), "safety");
    assert_eq!(s(last.get("action").unwrap(), "kind"), "stop");
    assert!(app.rpc("rideCoach", r#"{"trigger":"dance"}"#).is_err());
    call(&mut app, "stopSession", r#"{"save":true}"#);
    run(&mut app, 3000);
    assert!(app.rpc("rideCoach", r#"{"trigger":"how"}"#).is_err(), "no coach after the ride");
}

#[test]
fn ride_coach_ai_answers_off_the_control_path() {
    use std::io::{Read, Write};
    use std::sync::Mutex;
    // A fake local model server: answers every chat request with a ride reply.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let req_log = requests.clone();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { continue };
            let mut data = Vec::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = c.read(&mut buf).unwrap_or(0);
                data.extend_from_slice(&buf[..n]);
                let txt = String::from_utf8_lossy(&data).to_string();
                if let Some(h) = txt.find("\r\n\r\n") {
                    let len = txt[..h].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if data.len() >= h + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            req_log.lock().unwrap().push(String::from_utf8_lossy(&data).to_string());
            let content = r#"{\"schema\":\"ridgeline.ride.v1\",\"say\":\"Strong and steady, you're right on target.\",\"suggest\":\"harder\"}"#;
            let body = format!(r#"{{"model":"fake","choices":[{{"message":{{"role":"assistant","content":"{content}"}},"finish_reason":"stop"}}]}}"#);
            let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
        }
    });
    let dir = tmpdir();
    let app = Arc::new(Mutex::new(mk(&dir)));
    app.lock().unwrap().self_ref = Some(Arc::downgrade(&app));
    let tick = |ms: u64| {
        for _ in 0..ms / 100 {
            let mut a = app.lock().unwrap();
            a.advance_clock(100);
            a.tick();
        }
    };
    {
        let mut a = app.lock().unwrap();
        call(&mut a, "startDemo", "{}");
        let mut st = call(&mut a, "getSettings", "{}").get("settings").unwrap().clone();
        let mut ai = st.get("ai").unwrap().clone();
        ai.set("provider", "local");
        ai.set("base_url", format!("http://127.0.0.1:{port}/v1"));
        st.set("ai", ai);
        call(&mut a, "saveSettings", &Value::obj([("settings", st)]).to_string_compact());
        let mut prof = call(&mut a, "getProfile", "{}").get("profile").unwrap().clone();
        let mut consent = prof.get("ai_consent").unwrap().clone();
        consent.set("enabled", true);
        prof.set("ai_consent", consent);
        call(&mut a, "saveProfile", &Value::obj([("profile", prof)]).to_string_compact());
    }
    tick(4000);
    {
        let mut a = app.lock().unwrap();
        call(&mut a, "startSession", r#"{"mode":"erg","workout_id":"test-erg-short"}"#);
    }
    tick(30_000);
    let job = {
        let mut a = app.lock().unwrap();
        call(&mut a, "rideCoach", r#"{"trigger":"too_hard"}"#).get("job_id").unwrap().as_str().map(|x| x.to_string())
    };
    assert!(job.is_some(), "AI requests run as a background job");
    // The ride keeps ticking while the coach answers.
    let mut answered = None;
    for _ in 0..250 {
        tick(200);
        std::thread::sleep(std::time::Duration::from_millis(20)); // the job thread runs in real time
        let mut a = app.lock().unwrap();
        let f = feed(&mut a);
        if let Some(x) = f.iter().find(|x| s(x, "from") == "ai") {
            answered = Some(x.clone());
            break;
        }
    }
    let ai_item = answered.expect("AI reply arrives in the feed");
    assert_eq!(s(&ai_item, "text"), "Strong and steady, you're right on target.");
    // The model said "harder" after "too hard": dropped, and the easier option offered instead.
    let action = ai_item.get("action").unwrap();
    assert_eq!((s(action, "kind"), action.get("delta").unwrap().as_i64()), ("intensity".into(), Some(-5)));
    let sent = requests.lock().unwrap().join("\n");
    assert!(sent.contains("ridgeline.ride.v1") && sent.contains("RIDE"), "snapshot sent");
    assert!(!sent.contains("\"lat\"") && !sent.contains("\"lon\""), "no location in ride requests");
    let mut a = app.lock().unwrap();
    let st = call(&mut a, "getState", "{}");
    assert_eq!(st.get("session").unwrap().get("state").unwrap().as_str(), Some("running"));
}
