//! Small filesystem helpers: atomic writes, bounded reads, safe names.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_DOC_BYTES: u64 = 64 * 1024 * 1024;

/// Write via temp file + fsync + rename so readers never see a torn file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io_msg("create folder", dir, &e))?;
    }
    let tmp = path.with_extension(format!("{}tmp", path.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()));
    {
        let mut f = File::create(&tmp).map_err(|e| io_msg("create", &tmp, &e))?;
        f.write_all(bytes).map_err(|e| io_msg("write", &tmp, &e))?;
        f.sync_all().map_err(|e| io_msg("sync", &tmp, &e))?;
    }
    fs::rename(&tmp, path).map_err(|e| io_msg("rename", path, &e))?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

pub fn read_string(path: &Path) -> Result<Option<String>, String> {
    match File::open(path) {
        Ok(mut f) => {
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            if len > MAX_DOC_BYTES {
                return Err(format!("{} is too large to load", path.display()));
            }
            let mut s = String::new();
            f.read_to_string(&mut s).map_err(|e| io_msg("read", path, &e))?;
            Ok(Some(s))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_msg("open", path, &e)),
    }
}

pub fn append_line(path: &Path, line: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io_msg("create folder", dir, &e))?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path).map_err(|e| io_msg("open", path, &e))?;
    f.write_all(line.as_bytes()).and_then(|_| f.write_all(b"\n")).map_err(|e| io_msg("append", path, &e))
}

pub fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(p) else { return 0 };
    rd.filter_map(|e| e.ok())
        .map(|e| {
            let path = e.path();
            match e.metadata() {
                Ok(m) if m.is_dir() => dir_size(&path),
                Ok(m) => m.len(),
                Err(_) => 0,
            }
        })
        .sum()
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<u64, String> {
    fs::create_dir_all(to).map_err(|e| io_msg("create folder", to, &e))?;
    let mut n = 0;
    for e in fs::read_dir(from).map_err(|e| io_msg("read folder", from, &e))?.flatten() {
        let p = e.path();
        let dest = to.join(e.file_name());
        if p.is_dir() {
            n += copy_dir(&p, &dest)?;
        } else {
            n += fs::copy(&p, &dest).map_err(|e| io_msg("copy", &p, &e))?;
        }
    }
    Ok(n)
}

/// Identifiers used in paths must be UUIDs or slugs (no traversal).
pub fn safe_id(id: &str) -> Result<&str, String> {
    if rl_domain::ids::is_uuid(id) || rl_domain::ids::is_slug(id) {
        Ok(id)
    } else {
        Err("invalid identifier".into())
    }
}

pub fn io_msg(what: &str, path: &Path, e: &std::io::Error) -> String {
    let hint = match e.raw_os_error() {
        Some(28) | Some(112) => " (the disk is full)",
        _ => "",
    };
    format!("Could not {what} {}{hint}: {e}", short(path))
}

/// Path for messages: only the last two components (avoid leaking the user's home path into logs).
pub fn short(p: &Path) -> String {
    let comps: Vec<_> = p.components().rev().take(2).collect();
    let mut pb = PathBuf::new();
    for c in comps.into_iter().rev() {
        pb.push(c);
    }
    format!("…/{}", pb.display())
}
