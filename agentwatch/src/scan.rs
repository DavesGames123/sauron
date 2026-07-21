//! Incremental reader over the Claude Code session logs for one repo.
//!
//! The logs are append-only JSONL and reach 10MB. Re-parsing every file on every
//! 2s tick would burn real CPU alongside four running agents, so each file keeps
//! a byte offset and only newly appended bytes are parsed.
//!
//! grep targets:
//!   struct Scanner          -- owns per-file offsets and folded sessions
//!   fn Scanner::new         -- derives the log dir from a repo path
//!   fn Scanner::refresh     -- tail every jsonl, fold new records
//!   fn fold_record          -- one jsonl record -> mutation on a Session
//!   fn project_dir_for      -- /a/b/c -> ~/.claude/projects/-a-b-c

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{parse_rfc3339_ms, Session};

struct Tracked {
    /// Bytes already folded into `session`. Only complete lines are counted.
    offset: u64,
    session: Session,
}

pub struct Scanner {
    log_dir: PathBuf,
    repo_root: PathBuf,
    tracked: HashMap<PathBuf, Tracked>,
}

impl Scanner {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            log_dir: project_dir_for(&repo_root),
            repo_root,
            tracked: HashMap::new(),
        }
    }

    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Tail every session log in the directory and return the folded sessions.
    pub fn refresh(&mut self) -> Vec<Session> {
        let entries = match std::fs::read_dir(&self.log_dir) {
            Ok(e) => e,
            // Directory missing means no sessions for this repo yet -- not an error.
            Err(_) => return Vec::new(),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            self.tail_file(&path);
        }

        self.tracked.values().map(|t| t.session.clone()).collect()
    }

    fn tail_file(&mut self, path: &Path) {
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let entry = self.tracked.entry(path.to_path_buf()).or_insert_with(|| Tracked {
            offset: 0,
            session: Session {
                id,
                ..Default::default()
            },
        });

        let len = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };

        // Shrunk means the file was rewritten or rotated; the accumulated session
        // no longer describes it, so start clean rather than splicing garbage.
        if len < entry.offset {
            entry.offset = 0;
            let kept_id = entry.session.id.clone();
            entry.session = Session {
                id: kept_id,
                ..Default::default()
            };
        }
        if len == entry.offset {
            return;
        }

        let mut f = match File::open(path) {
            Ok(f) => f,
            Err(_) => return,
        };
        if f.seek(SeekFrom::Start(entry.offset)).is_err() {
            return;
        }

        let mut buf = Vec::with_capacity((len - entry.offset) as usize);
        if f.read_to_end(&mut buf).is_err() {
            return;
        }

        // A tick can land mid-write. Consume only through the last newline and
        // leave the partial tail for the next pass.
        let complete_to = match buf.iter().rposition(|b| *b == b'\n') {
            Some(i) => i + 1,
            None => return,
        };

        let text = String::from_utf8_lossy(&buf[..complete_to]).into_owned();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                fold_record(&mut entry.session, &v, &self.repo_root);
            }
        }

        entry.offset += complete_to as u64;
    }
}

/// Apply one log record to the session being accumulated.
fn fold_record(session: &mut Session, v: &Value, repo_root: &Path) {
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
        if let Some(ms) = parse_rfc3339_ms(ts) {
            if ms > session.last_activity {
                session.last_activity = ms;
            }
        }
    }

    if let Some(b) = v.get("gitBranch").and_then(|b| b.as_str()) {
        if !b.is_empty() {
            session.branch = Some(b.to_string());
        }
    }

    // Turn tracking. An assistant message carries `stop_reason`: `end_turn`
    // means it handed control back, anything else (`tool_use`, `max_tokens`)
    // means more is coming. A non-meta user record afterwards -- a tool result
    // or a fresh prompt -- puts the session back in flight.
    match kind {
        "assistant" => {
            let stop = v
                .get("message")
                .and_then(|m| m.get("stop_reason"))
                .and_then(|s| s.as_str());
            // `null` stop_reason appears on streaming partials; treat it as
            // in-flight rather than complete.
            session.turn_complete = stop == Some("end_turn");
        }
        "user" => {
            // isMeta records are harness bookkeeping (command caveats, hook
            // output), not the user or a tool actually driving the turn.
            if v.get("isMeta").and_then(|m| m.as_bool()) != Some(true) {
                session.turn_complete = false;
            }
        }
        _ => {}
    }

    fold_questions(session, v);

    match kind {
        "ai-title" => {
            if let Some(t) = v.get("aiTitle").and_then(|t| t.as_str()) {
                session.title = Some(t.to_string());
            }
        }
        "last-prompt" => {
            if let Some(p) = v.get("lastPrompt").and_then(|p| p.as_str()) {
                session.last_prompt = Some(p.to_string());
            }
        }
        // The write-set. This is the test surface.
        "file-history-delta" => {
            let Some(raw) = v.get("trackingPath").and_then(|p| p.as_str()) else {
                return;
            };
            let Some(rel) = repo_relative(raw, repo_root) else {
                return;
            };
            let ts = v
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(parse_rfc3339_ms)
                .unwrap_or(session.last_activity);
            let slot = session.edits.entry(rel).or_insert(ts);
            if ts > *slot {
                *slot = ts;
            }
        }
        _ => {}
    }
}

/// Track questions that stop the agent until a human answers.
///
/// `AskUserQuestion` and `ExitPlanMode` are logged as ordinary `tool_use`
/// blocks, and receive a matching `tool_result` only once answered. An
/// unmatched pair therefore means the agent is parked. Every other tool
/// resolves on its own, so only these two names are tracked -- a pending `Bash`
/// means "busy", not "blocked".
fn fold_questions(session: &mut Session, v: &Value) {
    const BLOCKING: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];

    let Some(content) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };

    for block in content {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("tool_use") => {
                let Some(id) = block.get("id").and_then(|i| i.as_str()) else {
                    continue;
                };
                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                // Every tool is tracked: an unresolved call plus a silent log is
                // the only trace a permission prompt leaves. The named two are
                // additionally certain to be waiting on a human.
                session.pending_tools.insert(id.to_string());
                if BLOCKING.contains(&name) {
                    session.open_questions.insert(id.to_string());
                }
            }
            Some("tool_result") => {
                if let Some(id) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                    session.open_questions.remove(id);
                    session.pending_tools.remove(id);
                }
            }
            _ => {}
        }
    }
}

/// Keep only writes that landed inside the repo.
///
/// Sessions also log writes to scratchpad scripts and `~/.claude` memory files.
/// Those are not testable software changes, and including them roughly doubled
/// the apparent write-set in every session sampled.
fn repo_relative(raw: &str, repo_root: &Path) -> Option<String> {
    let p = Path::new(raw);
    if p.is_absolute() {
        return p
            .strip_prefix(repo_root)
            .ok()
            .map(|r| r.to_string_lossy().into_owned());
    }
    Some(raw.to_string())
}

/// Claude Code encodes the project path by replacing separators with dashes:
/// `/Users/you/code/my-repo` -> `-Users-you-code-my-repo`.
pub fn project_dir_for(repo_root: &Path) -> PathBuf {
    let encoded = repo_root.to_string_lossy().replace(['/', '.'], "-");
    home().join(".claude").join("projects").join(encoded)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encodes_project_dir_like_claude_code() {
        let d = project_dir_for(Path::new("/Users/you/code/my-repo"));
        assert!(d.ends_with("-Users-you-code-my-repo"));
    }

    #[test]
    fn filters_write_set_to_repo_paths() {
        let root = Path::new("/Users/you/code/my-repo");
        // Relative paths are repo paths.
        assert_eq!(
            repo_relative("src/ecology/mod.rs", root).as_deref(),
            Some("src/ecology/mod.rs")
        );
        // Absolute inside the repo is stripped to relative.
        assert_eq!(
            repo_relative("/Users/you/code/my-repo/src/a.rs", root).as_deref(),
            Some("src/a.rs")
        );
        // Scratchpad and memory writes are not testable surface.
        assert!(repo_relative("/private/tmp/claude-501/x/scratchpad/cut.py", root).is_none());
        assert!(repo_relative("/Users/d/.claude/projects/x/memory/note.md", root).is_none());
    }

    #[test]
    fn folds_records_into_session() {
        let root = Path::new("/repo");
        let mut s = Session::default();

        fold_record(&mut s, &json!({"type":"ai-title","aiTitle":"Letters redesign"}), root);
        fold_record(
            &mut s,
            &json!({"type":"file-history-delta","trackingPath":"src/gui/letters.rs",
                    "timestamp":"2026-07-21T17:59:10.746Z"}),
            root,
        );
        fold_record(
            &mut s,
            &json!({"type":"assistant","timestamp":"2026-07-21T18:00:00.000Z",
                    "gitBranch":"station_physics"}),
            root,
        );

        assert_eq!(s.title.as_deref(), Some("Letters redesign"));
        assert_eq!(s.branch.as_deref(), Some("station_physics"));
        assert_eq!(s.edits.len(), 1);
        assert!(s.edits.contains_key("src/gui/letters.rs"));
        // last_activity tracks the newest record, not the newest edit.
        assert_eq!(
            s.last_activity,
            parse_rfc3339_ms("2026-07-21T18:00:00.000Z").unwrap()
        );
    }

    #[test]
    fn tracks_turn_completion_from_stop_reason() {
        let root = Path::new("/repo");
        let mut s = Session::default();

        // Mid-turn: assistant is about to run a tool.
        fold_record(
            &mut s,
            &json!({"type":"assistant","message":{"stop_reason":"tool_use"}}),
            root,
        );
        assert!(!s.turn_complete);

        // Handed control back.
        fold_record(
            &mut s,
            &json!({"type":"assistant","message":{"stop_reason":"end_turn"}}),
            root,
        );
        assert!(s.turn_complete);

        // Harness bookkeeping must not look like the user driving a new turn.
        fold_record(
            &mut s,
            &json!({"type":"user","isMeta":true,"message":{"role":"user"}}),
            root,
        );
        assert!(s.turn_complete, "isMeta record should not reopen the turn");

        // A real tool result or prompt does.
        fold_record(
            &mut s,
            &json!({"type":"user","message":{"role":"user"}}),
            root,
        );
        assert!(!s.turn_complete);
    }

    #[test]
    fn open_question_blocks_until_answered() {
        let root = Path::new("/repo");
        let mut s = Session::default();

        fold_record(
            &mut s,
            &json!({"type":"assistant","message":{"stop_reason":"tool_use","content":[
                {"type":"tool_use","id":"toolu_q1","name":"AskUserQuestion"}]}}),
            root,
        );
        assert_eq!(s.open_questions.len(), 1, "pending question is tracked");

        // The answer arrives as an ordinary tool_result carrying the same id.
        fold_record(
            &mut s,
            &json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"toolu_q1"}]}}),
            root,
        );
        assert!(s.open_questions.is_empty(), "answered question clears");
    }

    #[test]
    fn ordinary_tool_is_pending_but_not_a_certain_question() {
        let root = Path::new("/repo");
        let mut s = Session::default();
        fold_record(
            &mut s,
            &json!({"type":"assistant","message":{"stop_reason":"tool_use","content":[
                {"type":"tool_use","id":"toolu_b1","name":"Bash"}]}}),
            root,
        );
        // Not a question, but still unresolved -- a permission prompt on this
        // Bash call would look exactly like this in the log.
        assert!(s.open_questions.is_empty());
        assert_eq!(s.pending_tools.len(), 1);

        fold_record(
            &mut s,
            &json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"toolu_b1"}]}}),
            root,
        );
        assert!(s.pending_tools.is_empty(), "result clears the pending call");
    }

    #[test]
    fn exit_plan_mode_also_blocks() {
        let root = Path::new("/repo");
        let mut s = Session::default();
        fold_record(
            &mut s,
            &json!({"type":"assistant","message":{"stop_reason":"tool_use","content":[
                {"type":"tool_use","id":"toolu_p1","name":"ExitPlanMode"}]}}),
            root,
        );
        assert_eq!(s.open_questions.len(), 1);
    }

    #[test]
    fn later_edit_advances_path_timestamp() {
        let root = Path::new("/repo");
        let mut s = Session::default();
        let rec = |ts: &str| {
            json!({"type":"file-history-delta","trackingPath":"src/a.rs","timestamp":ts})
        };
        fold_record(&mut s, &rec("2026-07-21T10:00:00.000Z"), root);
        fold_record(&mut s, &rec("2026-07-21T12:00:00.000Z"), root);
        assert_eq!(
            s.edits["src/a.rs"],
            parse_rfc3339_ms("2026-07-21T12:00:00.000Z").unwrap()
        );
    }
}
