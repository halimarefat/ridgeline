//! Golden-packet fixtures (fixtures/packets/golden.json) through the real
//! parsers and encoders (acceptance A03). The JSON file is the shareable
//! record; this test keeps it honest.

use rl_device::bytes::{from_hex, hex};
use rl_device::ftms::{ControlCommand, ControlResponse, IndoorBikeData, SimParams, StopKind};
use rl_device::sensors::{CscMeasurement, CyclingPowerMeasurement, HeartRateMeasurement};
use rl_json::Value;

fn fixtures() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/packets/golden.json");
    rl_json::parse(&std::fs::read_to_string(path).expect("fixture file")).expect("valid JSON")
}

fn bytes(case: &Value) -> Vec<u8> {
    from_hex(case.get("hex").and_then(|h| h.as_str()).unwrap_or("")).expect("hex")
}

fn num<T: Into<f64>>(v: Option<T>) -> Value {
    v.map(|x| Value::from(x.into())).unwrap_or(Value::Null)
}

fn pair<A: Into<f64>, B: Into<f64>>(v: Option<(A, B)>) -> Value {
    v.map(|(a, b)| Value::Arr(vec![Value::from(a.into()), Value::from(b.into())])).unwrap_or(Value::Null)
}

/// Compare every expected key; numbers within 1e-9, null == absent.
fn check(name: &str, expect: &Value, actual: &[(&str, Value)]) {
    for (k, want) in expect.as_obj().expect("expect object") {
        let got = actual.iter().find(|(n, _)| n == k).map(|(_, v)| v).unwrap_or_else(|| panic!("{name}: unknown field {k}"));
        let ok = match (want, got) {
            (Value::Null, Value::Null) => true,
            (Value::Arr(a), Value::Arr(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() < 1e-9),
            (w, g) if w.as_f64().is_some() && g.as_f64().is_some() => (w.as_f64().unwrap() - g.as_f64().unwrap()).abs() < 1e-9,
            (w, g) => w == g,
        };
        assert!(ok, "{name}: field {k}: expected {want:?}, got {got:?}");
    }
}

fn cases<'a>(f: &'a Value, key: &str) -> &'a Vec<Value> {
    f.get(key).and_then(|c| c.as_arr()).unwrap_or_else(|| panic!("missing {key}"))
}

fn is_error(c: &Value) -> bool {
    c.get("error").and_then(|e| e.as_bool()) == Some(true)
}

fn name(c: &Value) -> String {
    c.get("name").and_then(|n| n.as_str()).unwrap_or("case").to_string()
}

#[test]
fn indoor_bike_data_fixtures() {
    let f = fixtures();
    for c in cases(&f, "indoor_bike_data") {
        let r = IndoorBikeData::parse(&bytes(c));
        if is_error(c) {
            assert!(r.is_err(), "{} should be rejected", name(c));
            continue;
        }
        let d = r.unwrap_or_else(|e| panic!("{}: {e:?}", name(c)));
        check(
            &name(c),
            c.get("expect").unwrap(),
            &[
                ("speed_kmh", num(d.speed_kmh)),
                ("avg_speed_kmh", num(d.avg_speed_kmh)),
                ("cadence_rpm", num(d.cadence_rpm)),
                ("avg_cadence_rpm", num(d.avg_cadence_rpm)),
                ("total_distance_m", num(d.total_distance_m)),
                ("resistance_level", num(d.resistance_level)),
                ("power_w", num(d.power_w)),
                ("avg_power_w", num(d.avg_power_w)),
                ("total_energy_kcal", num(d.total_energy_kcal)),
                ("energy_per_hour_kcal", num(d.energy_per_hour_kcal)),
                ("energy_per_min_kcal", num(d.energy_per_min_kcal)),
                ("heart_rate_bpm", num(d.heart_rate_bpm)),
                ("met", num(d.met)),
                ("elapsed_s", num(d.elapsed_s)),
                ("remaining_s", num(d.remaining_s)),
            ],
        );
    }
}

#[test]
fn heart_rate_fixtures() {
    let f = fixtures();
    for c in cases(&f, "heart_rate") {
        let r = HeartRateMeasurement::parse(&bytes(c));
        if is_error(c) {
            assert!(r.is_err(), "{} should be rejected", name(c));
            continue;
        }
        let m = r.unwrap();
        check(
            &name(c),
            c.get("expect").unwrap(),
            &[
                ("bpm", Value::from(m.bpm as f64)),
                ("contact_detected", m.contact_detected.map(Value::from).unwrap_or(Value::Null)),
                ("energy_expended_kj", num(m.energy_expended_kj)),
                ("rr_s", Value::Arr(m.rr_s.iter().map(|x| Value::from(*x)).collect())),
            ],
        );
    }
}

#[test]
fn cycling_power_and_csc_fixtures() {
    let f = fixtures();
    for c in cases(&f, "cycling_power") {
        let r = CyclingPowerMeasurement::parse(&bytes(c));
        if is_error(c) {
            assert!(r.is_err(), "{} should be rejected", name(c));
            continue;
        }
        let m = r.unwrap();
        check(
            &name(c),
            c.get("expect").unwrap(),
            &[("power_w", Value::from(m.power_w as f64)), ("wheel", pair(m.wheel)), ("crank", pair(m.crank)), ("pedal_balance_pct", num(m.pedal_balance_pct))],
        );
    }
    for c in cases(&f, "csc") {
        let m = CscMeasurement::parse(&bytes(c)).unwrap();
        check(&name(c), c.get("expect").unwrap(), &[("wheel", pair(m.wheel)), ("crank", pair(m.crank))]);
    }
}

#[test]
fn control_point_command_fixtures() {
    let f = fixtures();
    for c in cases(&f, "control_point_commands") {
        let n = |k: &str| c.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let cmd = match c.get("command").and_then(|v| v.as_str()).unwrap() {
            "set_target_power" => ControlCommand::SetTargetPower { watts: n("watts") as i16 },
            "set_target_resistance" => ControlCommand::SetTargetResistance { level: n("level") },
            "set_simulation" => ControlCommand::SetSimulation(SimParams { wind_speed_mps: n("wind_mps"), grade_percent: n("grade_pct"), crr: n("crr"), cw_kg_per_m: n("cw") }),
            "request_control" => ControlCommand::RequestControl,
            "start" => ControlCommand::StartOrResume,
            "stop" => ControlCommand::StopOrPause(StopKind::Stop),
            "pause" => ControlCommand::StopOrPause(StopKind::Pause),
            other => panic!("unknown command {other}"),
        };
        let r = cmd.encode();
        if is_error(c) {
            assert!(r.is_err(), "{cmd:?} should not encode");
            continue;
        }
        let b = r.unwrap();
        assert_eq!(hex(&b), c.get("hex").unwrap().as_str().unwrap(), "{cmd:?}");
        // The simulator decodes what Ridgeline writes: round trip.
        assert_eq!(ControlCommand::decode(&b).unwrap().encode().unwrap(), b);
    }
}

#[test]
fn control_point_response_fixtures() {
    let f = fixtures();
    for c in cases(&f, "control_point_responses") {
        let r = ControlResponse::parse(&bytes(c));
        if is_error(c) {
            assert!(r.is_err(), "{:?} should be rejected", c.get("hex"));
            continue;
        }
        let r = r.unwrap();
        check("response", c.get("expect").unwrap(), &[("request_opcode", Value::from(r.request_opcode as f64)), ("result", Value::from(r.result.label()))]);
    }
}
