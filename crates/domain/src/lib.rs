//! Ridgeline domain engine: workouts, plans, coaching policy, routes,
//! elevation processing and bicycle physics. Pure Rust standard library so it
//! builds and tests offline on every platform.

pub mod geo;
pub mod gpx;
pub mod ids;
pub mod library;
pub mod physics;
pub mod plan;
pub mod policy;
pub mod rider;
pub mod route;
pub mod time;
pub mod workout;
pub mod zones;
