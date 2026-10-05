# Fixtures

| Path | What | Checked by |
|---|---|---|
| `packets/golden.json` | Golden Bluetooth packets: FTMS Indoor Bike Data, Heart Rate, Cycling Power, CSC, FTMS control-point commands and responses, including truncated and out-of-range cases (A03) | `crates/device/tests/packet_fixtures.rs`, plus unit tests in `crates/device/src/{ftms,sensors}.rs` |
| `fit/synthetic-ride.fit`, `.csv`, `.expect.json` | A synthetic ride (10 minutes of riding with a 60 s pause, 2 laps, heart rate missing for the first 30 s) exported as FIT and CSV, with the expected decoded values (A14) | CI step "FIT export decoded by the Garmin FIT SDK" (`scripts/verify-fit.mjs`); regenerate with `scripts/rl.sh fit` |

Synthetic routes (the "Grade steps" A05 route and the 12 km demo loop) are generated in code (`crates/app/src/bundled.rs`, `rl_domain::route::synthetic`) so their profile is exact.

## Simulator fault scenarios

The deterministic simulator (`crates/device/src/simulator.rs`) implements the same adapter contract as Bluetooth. These scenarios run in CI:

| Scenario | Test |
|---|---|
| Scan, connect, subscribe, notify; trainer + HR + cadence + power at once | `manager::full_connection_flow_and_simultaneous_sensors`, `simulator::scan_connect_subscribe_and_notify` |
| Device held by another app | `manager::busy_device_reports_other_app` |
| Sensor stops sending, then reconnects | `manager::stale_sensor_and_reconnect` |
| Malformed packets counted, never used | `manager::malformed_packets_are_counted_not_used` |
| Control denied | `controller::denied_control_is_reported`, `simulator::control_requires_permission_and_honours_denial`, `session_sim::denied_control_returns_to_prepared` |
| Acknowledgement timeout → uncertain, no blind retry | `controller::timeouts_make_state_uncertain_without_blind_retry` |
| Newest target wins, rate limit | `controller::newest_target_wins_and_rate_limited` |
| Stop with and without acknowledgement | `controller::stop_path_discards_targets_and_confirms`, `controller::stop_without_ack_is_not_reported_as_success` |
| Stale-generation commands dropped (A08) | `controller::stale_generation_commands_are_dropped`, `session_sim::a08_mode_switch_leaves_one_controller_and_drops_stale_commands` |
| Unsupported mode, power/grade saturation | `controller::unsupported_mode_and_saturation`, `simulator::grade_saturates_at_device_limit` |
| Disconnect under load (A09) | `controller::disconnect_under_load_requires_new_grant_and_no_replay`, `session_sim::a09_disconnect_under_load_requires_controlled_resumption` |
| ERG workout: warm-up, intervals, pause/resume, finish (A04 logic) | `session_sim::a04_erg_workout_runs_pauses_and_finishes` |
| Road grades flat / +5 % / −3 % (A05) | `session_sim::a05_free_ride_sends_signed_grades` |
| Low-cadence protection | `session_sim::low_cadence_eases_load_and_recovers_with_rider_confirmation` |
| Two-hour session: no drift or duplicated samples (A16 logic) | `session_sim::a16_long_session_has_no_drift_or_duplication` |

The same faults can be toggled by hand in the app: Devices → Simulator (demo mode).
