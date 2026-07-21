//! Session model and time helpers.
//!
//! grep targets:
//!   struct Session          -- one Claude Code session, folded from its jsonl
//!   fn Session::status      -- Working / NeedsTest / Clear derivation
//!   fn Session::pending     -- write-set entries not yet acked at their current ts
//!   fn parse_rfc3339_ms     -- ISO8601 -> epoch millis, no chrono dependency
//!   fn ago                  -- epoch millis -> "4m" / "2h" / "3d"

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

/// Escape hatch for a session that died mid-turn.
///
/// `turn_complete` is the real signal, but an agent killed between a `tool_use`
/// and its result never logs an `end_turn`, so it would read as Working forever
/// and its writes would never surface for testing. Past this much silence,
/// assume the turn will never finish.
pub const STUCK_AFTER_MS: i64 = 30 * 60 * 1000;

/// Sessions quieter than this with nothing pending are hidden entirely.
pub const DORMANT_AFTER_MS: i64 = 24 * 60 * 60 * 1000;

/// Untested writes older than this are collapsed behind a counter rather than
/// listed. Without it the first launch shows every session ever run against the
/// repo -- 44 of them here -- which reproduces the overload this tool exists to
/// remove. Toggle with `o`, clear permanently with `--baseline`.
pub const STALE_HORIZON_MS: i64 = 12 * 60 * 60 * 1000;

/// A session with an unresolved tool call and no log activity for this long is
/// treated as wanting a human.
///
/// Claude Code never logs a permission prompt -- "Do you want to proceed?" is
/// live UI state that only reaches the log once answered -- so the sole
/// observable is a `tool_use` with no `tool_result` and a silent log. A long
/// `cargo build` produces the same shape, so this deliberately over-reports:
/// a false "check this one" costs a glance, while a false "still working" hides
/// an agent parked on an approval indefinitely, which is the failure that
/// actually wastes the day.
pub const STALL_AFTER_MS: i64 = 45_000;

/// Why a session is waiting on the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedReason {
    /// AskUserQuestion / ExitPlanMode with no result: certain.
    Question,
    /// Unresolved tool call plus a silent log: probable permission prompt, but
    /// indistinguishable from a genuinely slow command.
    MaybeApproval,
}

impl BlockedReason {
    pub fn detail(self) -> &'static str {
        match self {
            BlockedReason::Question => {
                "agent asked a question and is stopped until you answer"
            }
            BlockedReason::MaybeApproval => {
                "tool call unresolved and log quiet — likely a permission prompt, or a slow command"
            }
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            BlockedReason::Question => "asked you a question",
            BlockedReason::MaybeApproval => "may need approval",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Asked a question (AskUserQuestion / ExitPlanMode) that has no tool_result
    /// yet. The agent has stopped and is burning wall-clock waiting on a human.
    /// Ranks above everything else: nothing else on screen is costing time right
    /// now the way this is.
    Blocked,
    /// Mid-turn -- the agent is computing.
    Working,
    /// Idle, and has repo edits you have not acked at their current timestamp.
    NeedsTest,
    /// Idle with nothing outstanding.
    Clear,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Blocked => "NEEDS YOUR ANSWER",
            Status::Working => "working",
            Status::NeedsTest => "NEEDS TEST",
            Status::Clear => "clear",
        }
    }

    /// Sort rank: what should demand attention first. A blocked agent outranks
    /// untested work because it is stalled until you act; untested work is not.
    pub fn rank(self) -> u8 {
        match self {
            Status::Blocked => 0,
            Status::NeedsTest => 1,
            Status::Working => 2,
            Status::Clear => 3,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    pub branch: Option<String>,
    /// Latest timestamp seen on any record, epoch millis.
    pub last_activity: i64,
    /// True when the last assistant message ended with `stop_reason: end_turn`
    /// and no user turn followed. This -- not elapsed time -- is what makes a
    /// session's writes safe to test: an agent can sit for ten minutes inside a
    /// single tool call, and a timeout would call that "done" while it is still
    /// editing the very files you would go test.
    pub turn_complete: bool,
    /// tool_use ids of AskUserQuestion / ExitPlanMode calls with no tool_result
    /// yet. Non-empty means the agent is parked on a question.
    pub open_questions: BTreeSet<String>,
    /// tool_use ids of any kind with no tool_result yet. Combined with a silent
    /// log this is the only available trace of a pending permission prompt.
    pub pending_tools: BTreeSet<String>,
    /// Repo-relative path -> epoch millis of its most recent write by this session.
    pub edits: BTreeMap<String, i64>,
}

impl Session {
    /// First 8 chars of the uuid -- enough to disambiguate, short enough to scan.
    pub fn short_id(&self) -> &str {
        let n = self.id.len().min(8);
        &self.id[..n]
    }

    /// Title if the session has earned one, else a trimmed last prompt, else the id.
    pub fn display_name(&self) -> String {
        if let Some(t) = &self.title {
            if !t.trim().is_empty() {
                return t.clone();
            }
        }
        if let Some(p) = &self.last_prompt {
            let one = collapse_ws(p);
            if !one.is_empty() {
                return truncate(&one, 60);
            }
        }
        self.short_id().to_string()
    }

    /// Edits whose current timestamp is newer than what was acked for that path.
    /// A path acked and then re-edited reappears here -- that is the whole point.
    ///
    /// Writes older than `STALE_HORIZON_MS` drop out on their own. Acking must
    /// stay optional: if the only way to clear the board were to press `a` on
    /// every session, the tool would replace "remember what to test" with
    /// "remember to file paperwork", which is the same overhead wearing a
    /// different hat. Ack is for saying "checked this one now"; anything you
    /// never got to simply ages out.
    pub fn pending<'a>(
        &'a self,
        acked: Option<&BTreeMap<String, i64>>,
        now: i64,
    ) -> Vec<&'a str> {
        let cutoff = now.saturating_sub(STALE_HORIZON_MS);
        self.edits
            .iter()
            .filter(|(_, ts)| **ts >= cutoff)
            .filter(|(path, ts)| match acked.and_then(|a| a.get(path.as_str())) {
                Some(acked_ts) => *ts > acked_ts,
                None => true,
            })
            .map(|(path, _)| path.as_str())
            .collect()
    }

    /// Why this session is waiting on the user, if it is.
    pub fn blocked_reason(&self, now: i64) -> Option<BlockedReason> {
        if !self.open_questions.is_empty() {
            return Some(BlockedReason::Question);
        }
        // Nothing logged for a while with a tool call still open. Most often a
        // permission prompt sitting unanswered on some other terminal.
        if !self.pending_tools.is_empty()
            && now.saturating_sub(self.last_activity) > STALL_AFTER_MS
        {
            return Some(BlockedReason::MaybeApproval);
        }
        None
    }

    pub fn status(&self, now: i64, acked: Option<&BTreeMap<String, i64>>) -> Status {
        // A session waiting on a human outranks everything: it is doing nothing
        // at all until someone replies, so this is the only state where the
        // delay is entirely the user's to remove.
        if self.blocked_reason(now).is_some() {
            return Status::Blocked;
        }
        // Mid-turn means the write set is still moving. Reporting it as testable
        // is the dangerous direction to be wrong in -- you go exercise a file the
        // agent is halfway through rewriting -- so only an explicit end_turn (or
        // a stuck-turn timeout) clears a session for testing.
        let stuck = now.saturating_sub(self.last_activity) > STUCK_AFTER_MS;
        if !self.turn_complete && !stuck {
            return Status::Working;
        }
        if self.pending(acked, now).is_empty() {
            Status::Clear
        } else {
            Status::NeedsTest
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Parse `2026-07-21T17:59:10.746Z` to epoch millis.
///
/// Hand-rolled rather than pulling chrono: the format is fixed by the log writer,
/// and a 30-line function is cheaper than a dependency tree in a sidecar. Returns
/// None on anything that does not match, so a malformed record is skipped rather
/// than poisoning the session's activity clock.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse::<i64>().ok() };

    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);

    // Fractional seconds are optional and of unspecified length.
    let ms = if b.len() > 20 && b[19] == b'.' {
        let frac: String = s[20..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(3)
            .collect();
        let scaled = format!("{:0<3}", frac);
        scaled.parse::<i64>().unwrap_or(0)
    } else {
        0
    };

    let days = days_from_civil(y, mo, d);
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec) * 1000) + ms)
}

/// Howard Hinnant's days_from_civil: civil date -> days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Compact relative age: "12s", "4m", "2h", "3d".
pub fn ago(then_ms: i64, now_ms: i64) -> String {
    let s = (now_ms.saturating_sub(then_ms) / 1000).max(0);
    if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_timestamp_format() {
        // The exact shape emitted into the session jsonl.
        let ms = parse_rfc3339_ms("2026-07-21T17:59:10.746Z").unwrap();
        // 1970-01-01 sanity: round-trips through the same civil conversion.
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00.000Z").unwrap(), 0);
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:01.500Z").unwrap(), 1500);
        // Monotonic within the same day.
        let later = parse_rfc3339_ms("2026-07-21T18:00:00.000Z").unwrap();
        assert!(later > ms);
        // Missing fractional part is tolerated.
        assert!(parse_rfc3339_ms("2026-07-21T17:59:10Z").is_some());
        // Garbage is rejected rather than silently zeroed.
        assert!(parse_rfc3339_ms("not-a-timestamp").is_none());
    }

    #[test]
    fn reedited_path_becomes_pending_again() {
        let now = 1_000_000_000i64;
        let mut s = Session::default();
        s.edits.insert("src/a.rs".into(), now - 1000);

        let mut acked = BTreeMap::new();
        acked.insert("src/a.rs".to_string(), now - 1000);
        assert!(
            s.pending(Some(&acked), now).is_empty(),
            "acked at current ts"
        );

        // Agent rewrites the same file after the ack.
        s.edits.insert("src/a.rs".into(), now - 500);
        assert_eq!(
            s.pending(Some(&acked), now),
            vec!["src/a.rs"],
            "re-edit after ack must resurface"
        );
    }

    #[test]
    fn unacked_writes_expire_without_any_keypress() {
        let now = 1_000_000_000i64;
        let mut s = Session::default();
        s.turn_complete = true;

        // Written just now, never acked: outstanding.
        s.edits.insert("src/a.rs".into(), now - 1000);
        assert_eq!(s.status(now, None), Status::NeedsTest);

        // Same write, once it is older than the horizon: clears itself. Acking
        // is for "I checked this"; it must never be the only way off the list,
        // or the tool just trades one bookkeeping chore for another.
        let later = now + STALE_HORIZON_MS + 1;
        assert!(s.pending(None, later).is_empty());
        assert_eq!(s.status(later, None), Status::Clear);
    }

    #[test]
    fn unresolved_tool_plus_silence_reads_as_waiting_on_you() {
        let now = 1_000_000_000i64;
        let mut s = Session::default();
        s.last_activity = now;
        s.pending_tools.insert("toolu_1".into());

        // Just issued: the tool is presumably running.
        assert_eq!(s.blocked_reason(now), None);
        assert_eq!(s.status(now, None), Status::Working);

        // Still unresolved and nothing logged since: almost always a permission
        // prompt sitting on another terminal.
        let later = now + STALL_AFTER_MS + 1;
        assert_eq!(
            s.blocked_reason(later),
            Some(BlockedReason::MaybeApproval)
        );
        assert_eq!(s.status(later, None), Status::Blocked);
    }

    #[test]
    fn a_confirmed_question_outranks_a_guessed_approval() {
        let now = 1_000_000_000i64;
        let mut s = Session::default();
        s.last_activity = now - STALL_AFTER_MS - 1;
        s.pending_tools.insert("toolu_1".into());
        s.open_questions.insert("toolu_1".into());
        // Both conditions hold; report the one we are certain about.
        assert_eq!(s.blocked_reason(now), Some(BlockedReason::Question));
    }

    #[test]
    fn resolved_tools_leave_the_session_working() {
        let now = 1_000_000_000i64;
        let mut s = Session::default();
        s.last_activity = now - STALL_AFTER_MS - 1;
        // Nothing pending: quiet just means between turns, not blocked.
        assert_eq!(s.blocked_reason(now), None);
    }

    #[test]
    fn mid_turn_session_is_never_reported_testable() {
        let mut s = Session::default();
        s.edits.insert("src/a.rs".into(), 1);
        s.last_activity = 1_000_000;
        s.turn_complete = false;

        // The old timeout rule flipped to NeedsTest after two minutes of quiet.
        // An agent thinking, compiling, or inside one long tool call routinely
        // exceeds that while still rewriting the files you would go test.
        assert_eq!(s.status(1_000_000, None), Status::Working);
        assert_eq!(s.status(1_000_000 + 10 * 60 * 1000, None), Status::Working);
    }

    #[test]
    fn completed_turn_is_testable_immediately() {
        let mut s = Session::default();
        s.edits.insert("src/a.rs".into(), 1);
        s.last_activity = 1_000_000;
        s.turn_complete = true;
        // No waiting period: end_turn means the write set has settled.
        assert_eq!(s.status(1_000_000, None), Status::NeedsTest);
    }

    #[test]
    fn stuck_turn_eventually_releases() {
        let mut s = Session::default();
        s.edits.insert("src/a.rs".into(), 1);
        s.last_activity = 1_000_000;
        s.turn_complete = false;

        // An agent killed between tool_use and its result never logs end_turn.
        // Without the escape hatch its writes would be invisible forever.
        assert_eq!(
            s.status(1_000_000 + STUCK_AFTER_MS - 1, None),
            Status::Working
        );
        assert_eq!(
            s.status(1_000_000 + STUCK_AFTER_MS + 1, None),
            Status::NeedsTest
        );
    }
}
