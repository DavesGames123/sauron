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
}

impl AckStore {
    pub fn load() -> Self {
        let path = home().join(".claude").join("agentwatch").join("acks.json");
        let acks = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|v| decode(&v))
            // Absent on first run; corrupt means someone hand-edited it. Either
            // way an empty store is recoverable -- everything just reads as
            // untested, which is the safe direction to fail.
            .unwrap_or_default();
        Self { path, acks }
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

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let encoded = encode(&self.acks);
        let body = serde_json::to_string_pretty(&encoded)?;

        // Write-then-rename: a crash mid-save must not leave a truncated file
        // that reads as "everything untested" on next launch.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &self.path)
    }
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
    fn malformed_state_decodes_to_empty_not_panic() {
        assert!(decode(&Value::String("junk".into())).is_empty());
        assert!(decode(&serde_json::json!({"s": 5})).is_empty());
        // Non-integer timestamps are dropped rather than defaulted to 0, which
        // would otherwise read as "acked long ago" and hide real edits.
        let d = decode(&serde_json::json!({"s": {"a.rs": "nope"}}));
        assert!(d["s"].is_empty());
    }
}
