//! Background jobs (AI requests, routing, elevation sampling). They run on
//! their own threads, report progress, can be cancelled, and never hold the
//! application lock while waiting on the network.

use rl_json::{ToJson, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: String,
    pub kind: String,
    /// "running", "done", "failed", "cancelled"
    pub state: String,
    pub progress: Option<(usize, usize)>,
    pub message: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub cancel: Arc<AtomicBool>,
    pub created_ms: u64,
    pub finished_ms: Option<u64>,
}

impl JobInfo {
    pub fn to_json(&self) -> Value {
        Value::obj([
            ("id", self.id.clone().into()),
            ("kind", self.kind.clone().into()),
            ("state", self.state.clone().into()),
            ("progress", self.progress.map(|(a, b)| Value::Arr(vec![a.into(), b.into()])).unwrap_or(Value::Null)),
            ("message", self.message.clone().into()),
            ("result", self.result.clone().unwrap_or(Value::Null)),
            ("error", self.error.clone().into()),
        ])
    }
}

#[derive(Default)]
pub struct Jobs {
    pub map: BTreeMap<String, JobInfo>,
}

impl Jobs {
    pub fn insert(&mut self, j: JobInfo) {
        self.map.insert(j.id.clone(), j);
        // Keep the 30 most recent finished jobs.
        let mut finished: Vec<(u64, String)> = self.map.values().filter(|j| j.state != "running").map(|j| (j.finished_ms.unwrap_or(0), j.id.clone())).collect();
        if finished.len() > 30 {
            finished.sort();
            for (_, id) in finished.iter().take(finished.len() - 30) {
                self.map.remove(id);
            }
        }
    }
    pub fn running(&self, kind: &str) -> Option<&JobInfo> {
        self.map.values().find(|j| j.kind == kind && j.state == "running")
    }
    pub fn to_json(&self) -> Value {
        Value::Arr(self.map.values().map(|j| j.to_json()).collect())
    }
    pub fn cancel(&mut self, id: &str) -> bool {
        match self.map.get(id) {
            Some(j) if j.state == "running" => {
                j.cancel.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }
}

/// Handle given to a job thread.
pub struct JobCtx {
    pub id: String,
    pub cancel: Arc<AtomicBool>,
    pub app: Weak<Mutex<crate::App>>,
}

impl JobCtx {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn progress(&self, done: usize, total: usize, msg: &str) {
        if let Some(a) = self.app.upgrade() {
            if let Ok(mut a) = a.lock() {
                if let Some(j) = a.jobs.map.get_mut(&self.id) {
                    j.progress = Some((done, total));
                    j.message = msg.to_string();
                }
            }
        }
    }
    /// Run `f` with the application locked (for short state updates only).
    pub fn with_app<R>(&self, f: impl FnOnce(&mut crate::App) -> R) -> Option<R> {
        let a = self.app.upgrade()?;
        let mut g = a.lock().ok()?;
        Some(f(&mut g))
    }
}

pub fn job_result_to_json<T: ToJson>(t: &T) -> Value {
    t.to_json()
}
