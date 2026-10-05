//! Append-only activity journal (crash-safe recording).
//!
//! Samples, events and laps are appended as JSON lines. Buffered lines are
//! written to the OS at least every second and fsync'd at least every two
//! seconds, so a process crash loses at most ~1 s and a power loss at most
//! ~2–3 s of samples. Lines that fail to write (e.g. disk full) stay queued
//! in memory and are retried; the session shows the error.

use crate::fsx::io_msg;
use rl_json::ToJson;
use rl_session::record::{Lap, RecordSink, Sample, SessionEvent};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const FLUSH_EVERY: Duration = Duration::from_millis(1000);
pub const FSYNC_EVERY: Duration = Duration::from_millis(2000);
const MAX_QUEUED_LINES: usize = 50_000;

struct Stream {
    path: PathBuf,
    file: Option<File>,
    queued: Vec<String>,
}

impl Stream {
    fn open(path: PathBuf) -> Stream {
        Stream { path, file: None, queued: Vec::new() }
    }
    fn write_out(&mut self) -> Result<bool, String> {
        if self.queued.is_empty() {
            return Ok(false);
        }
        if self.file.is_none() {
            let f = OpenOptions::new().create(true).append(true).open(&self.path).map_err(|e| io_msg("open", &self.path, &e))?;
            self.file = Some(f);
        }
        let mut buf = String::with_capacity(self.queued.iter().map(|l| l.len() + 1).sum());
        for l in &self.queued {
            buf.push_str(l);
            buf.push('\n');
        }
        let f = self.file.as_mut().unwrap();
        match f.write_all(buf.as_bytes()) {
            Ok(()) => {
                self.queued.clear();
                Ok(true)
            }
            Err(e) => {
                // A partial write may have landed; recovery tolerates a torn last line.
                self.file = None;
                Err(io_msg("write", &self.path, &e))
            }
        }
    }
    fn sync(&mut self) -> Result<(), String> {
        if let Some(f) = self.file.as_mut() {
            f.sync_data().map_err(|e| io_msg("sync", &self.path, &e))?;
        }
        Ok(())
    }
}

pub struct ActivityJournal {
    pub dir: PathBuf,
    samples: Stream,
    events: Stream,
    laps: Stream,
    last_flush: Instant,
    last_sync: Instant,
    dirty_since_sync: bool,
    pub error: Option<String>,
    pub lines_written: u64,
}

impl ActivityJournal {
    pub fn create(dir: &Path) -> Result<ActivityJournal, String> {
        std::fs::create_dir_all(dir).map_err(|e| io_msg("create folder", dir, &e))?;
        Ok(ActivityJournal {
            dir: dir.to_path_buf(),
            samples: Stream::open(dir.join("samples.jsonl")),
            events: Stream::open(dir.join("events.jsonl")),
            laps: Stream::open(dir.join("laps.jsonl")),
            last_flush: Instant::now(),
            last_sync: Instant::now(),
            dirty_since_sync: false,
            error: None,
            lines_written: 0,
        })
    }

    fn queue(stream: &mut Stream, line: String) -> Result<(), String> {
        if stream.queued.len() >= MAX_QUEUED_LINES {
            return Err("Recording buffer is full: storage has been failing for a long time.".into());
        }
        stream.queued.push(line);
        Ok(())
    }

    fn write_all(&mut self) -> Result<(), String> {
        let mut wrote = false;
        let mut err = None;
        for s in [&mut self.samples, &mut self.events, &mut self.laps] {
            match s.write_out() {
                Ok(w) => wrote |= w,
                Err(e) => err = Some(e),
            }
        }
        if wrote {
            self.dirty_since_sync = true;
        }
        match err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl RecordSink for ActivityJournal {
    fn sample(&mut self, s: &Sample) -> Result<(), String> {
        self.lines_written += 1;
        Self::queue(&mut self.samples, s.to_json().to_string_compact())
    }
    fn event(&mut self, e: &SessionEvent) -> Result<(), String> {
        Self::queue(&mut self.events, e.to_json().to_string_compact())?;
        // Pause/stop events are important for recovery: write promptly.
        if matches!(e.kind.as_str(), "start" | "pause" | "stop") {
            self.flush(true)?;
        }
        Ok(())
    }
    fn lap(&mut self, l: &Lap) -> Result<(), String> {
        Self::queue(&mut self.laps, l.to_json().to_string_compact())
    }
    fn flush(&mut self, force: bool) -> Result<(), String> {
        let now = Instant::now();
        let mut result = Ok(());
        if force || now.duration_since(self.last_flush) >= FLUSH_EVERY {
            self.last_flush = now;
            result = self.write_all();
        }
        if self.dirty_since_sync && (force || now.duration_since(self.last_sync) >= FSYNC_EVERY) {
            self.last_sync = now;
            self.dirty_since_sync = false;
            for s in [&mut self.samples, &mut self.events, &mut self.laps] {
                if let Err(e) = s.sync() {
                    result = Err(e);
                }
            }
        }
        match &result {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(e.clone()),
        }
        result
    }
}

/// Read a JSONL file, tolerating a torn final line (crash mid-write).
pub fn read_jsonl(path: &Path) -> (Vec<rl_json::Value>, usize) {
    let Ok(Some(s)) = crate::fsx::read_string(path) else { return (vec![], 0) };
    let mut out = Vec::new();
    let mut bad = 0;
    for line in s.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match rl_json::parse(line) {
            Ok(v) => out.push(v),
            Err(_) => bad += 1,
        }
    }
    (out, bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_json::FromJson;

    #[test]
    fn journal_survives_torn_line() {
        let dir = std::env::temp_dir().join(format!("rl-journal-{}", rl_domain::ids::new_uuid()));
        let mut j = ActivityJournal::create(&dir).unwrap();
        for i in 1..=5u32 {
            j.sample(&Sample { active_s: i, power: Some(100.0 + i as f64), ..Default::default() }).unwrap();
        }
        j.flush(true).unwrap();
        // Simulate a crash mid-write.
        crate::fsx::append_line(&dir.join("samples.jsonl"), "{\"t\":6000,\"u\":0,\"a\":6,\"p\":10").unwrap();
        let (rows, bad) = read_jsonl(&dir.join("samples.jsonl"));
        assert_eq!(rows.len(), 5);
        assert_eq!(bad, 1);
        assert_eq!(Sample::from_json(&rows[4]).unwrap().power, Some(105.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
