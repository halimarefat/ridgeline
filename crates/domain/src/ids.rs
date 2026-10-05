//! Random identifiers.
//!
//! UUID v4 strings generated from the standard library's per-process random
//! hasher keys (seeded from the OS RNG) mixed with a counter and the clock.
//! These are identifiers, not secrets.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_u64() -> u64 {
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_i64(crate::time::now_utc_ms());
    h.finish()
}

pub fn new_uuid() -> String {
    let a = random_u64();
    let b = random_u64();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Accept only canonical UUID text; used to validate IDs arriving over IPC
/// before they are used as file names.
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Safe identifier for built-in content and file names: [a-z0-9-_], 1..64 chars.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

/// Either a UUID or a slug (built-in content ids).
pub fn is_safe_id(s: &str) -> bool {
    is_uuid(s) || is_slug(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uuid_shape_and_uniqueness() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let u = new_uuid();
            assert!(is_uuid(&u), "{u}");
            assert_eq!(&u[14..15], "4");
            assert!(seen.insert(u));
        }
        assert!(!is_uuid("../../etc/passwd"));
        assert!(is_slug("sweet-spot-3x10"));
        assert!(!is_slug("../x"));
        assert!(!is_safe_id("a/b"));
    }
}
