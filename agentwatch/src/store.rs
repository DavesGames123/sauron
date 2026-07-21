//! Ack persistence.
//!
//! Stores `session id -> { repo path -> edit timestamp acked }`. Storing the
//! timestamp rather than a bare path set is load-bearing: if it stored only
//! paths, a file you acked and an agent then rewrote would stay silently green,
//! which is the exact failure this tool exists to prevent.
//!
//! Lives under ~/.claude, never inside the repo -- the sidecar must not add
//! untracked files to a working tree that is already dirty.
//!
//! grep targets:
//!   struct AckStore     -- in-memory map plus its on-disk path
//!   fn AckStore::load   -- read, tolerating absent or corrupt state
//!   fn AckStore::ack    -- record every current edit ts for one session
//!   fn AckStore::save   -- atomic write via temp file + rename

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::scan::home;

pub type PathAcks = BTreeMap<String, i64>;

pub struct AckStore {
    path: PathBuf,
    acks: BTreeMap<String, PathAcks>,
    /// session id -> the last_activity it had when the user dismissed its
    /// "waiting on you" state. It stays dismissed until the session logs
    /// something newer, at which point it has done fresh work and re-surfaces.
    /// Kept in a sibling file so the acks format is untouched.
    dismissed_path: PathBuf,
    dismissed: BTreeMap<String, i64>,
}

impl AckStore {
    pub fn load() -> Self {
        let dir = home().join(".claude").join("agentwatch");
        let path = dir.join("acks.json");
        let dismissed_path = dir.join("dismissed.json");
        let acks = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|v| decode(&v))
            // Absent on first run; corrupt means someone hand-edited it. Either
            // way an empty store is recoverable -- everything just reads as
            // untested, which is the safe direction to fail.
            .unwrap_or_default();
        let dismissed = std::fs::read_to_string(&dismissed_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|v| decode_flat(&v))
            .unwrap_or_default();
        Self {
            path,
            acks,
            dismissed_path,
            dismissed,
        }
    }

    pub fn for_session(&self, id: &str) -> Option<&PathAcks> {
        self.acks.get(id)
    }

    pub fn session_count(&self) -> usize {
        self.acks.len()
    }

    /// Mark this session's write-set as tested at its current timestamps.
    pub fn ack(&mut self, id: &str, edits: &BTreeMap<String, i64>) {
        let slot = self.acks.entry(id.to_string()).or_default();
        for (path, ts) in edits {
            slot.insert(path.clone(), *ts);
        }
    }

    /// Drop a session's acks so its whole write-set reads as untested again.
    pub fn unack(&mut self, id: &str) {
        self.acks.remove(id);
    }

    /// Dismiss a session's current "waiting on you" state. Recording the
    /// activity timestamp -- not a bare flag -- is what makes it re-surface the
    /// moment the agent does anything new, so a dismissed session that then asks
    /// a fresh question is not silently hidden.
    pub fn dismiss(&mut self, id: &str, last_activity: i64) {
        self.dismissed.insert(id.to_string(), last_activity);
    }

    pub fn undismiss(&mut self, id: &str) {
        self.dismissed.remove(id);
    }

    /// The activity timestamp at which this session was dismissed, if it was.
    pub fn dismissed_at(&self, id: &str) -> Option<i64> {
        self.dismissed.get(id).copied()
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(&self.path, &encode(&self.acks))?;
        write_atomic(&self.dismissed_path, &encode_flat(&self.dismissed))
    }
}

/// Write-then-rename: a crash mid-save must not leave a truncated file that
/// reads as "everything untested" on next launch.
fn write_atomic(path: &std::path::Path, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_string_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

fn decode_flat(v: &Value) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    if let Some(obj) = v.as_object() {
        for (id, ts) in obj {
            if let Some(ts) = ts.as_i64() {
                out.insert(id.clone(), ts);
            }
        }
    }
    out
}

fn encode_flat(map: &BTreeMap<String, i64>) -> Value {
    let mut root = Map::new();
    for (id, ts) in map {
        root.insert(id.clone(), Value::from(*ts));
    }
    Value::Object(root)
}

fn decode(v: &Value) -> BTreeMap<String, PathAcks> {
    let mut out = BTreeMap::new();
    let Some(obj) = v.as_object() else {
        return out;
    };
    for (session, paths) in obj {
        let Some(paths) = paths.as_object() else {
            continue;
        };
        let mut inner = PathAcks::new();
        for (path, ts) in paths {
            if let Some(ts) = ts.as_i64() {
                inner.insert(path.clone(), ts);
            }
        }
        out.insert(session.clone(), inner);
    }
    out
}

fn encode(acks: &BTreeMap<String, PathAcks>) -> Value {
    let mut root = Map::new();
    for (session, paths) in acks {
        let mut inner = Map::new();
        for (path, ts) in paths {
            inner.insert(path.clone(), Value::from(*ts));
        }
        root.insert(session.clone(), Value::Object(inner));
    }
    Value::Object(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let mut acks: BTreeMap<String, PathAcks> = BTreeMap::new();
        let mut inner = PathAcks::new();
        inner.insert("src/a.rs".into(), 1_700_000_000_000);
        acks.insert("sess-1".into(), inner);

        let decoded = decode(&encode(&acks));
        assert_eq!(decoded, acks);
    }

    #[test]
    fn dismissed_map_round_trips() {
        let mut m: BTreeMap<String, i64> = BTreeMap::new();
        m.insert("sess-1".into(), 1_700_000_000_000);
        assert_eq!(decode_flat(&encode_flat(&m)), m);
        // Non-integer timestamps are dropped, not defaulted.
        assert!(decode_flat(&serde_json::json!({"s": "nope"})).is_empty());
    }

    #[test]
    fn malformed_state_decodes_to_empty_not_panic() {
        assert!(decode(&Value::String("junk".into())).is_empty());
        assert!(decode(&serde_json::json!({"s": 5})).is_empty());
        // Non-integer timestamps are dropped rather than defaulted to 0, which
        // would otherwise read as "acked long ago" and hide real edits.
        let d = decode(&serde_json::json!({"s": {"a.rs": "nope"}}));
        assert!(d["s"].is_empty());
    }
}
