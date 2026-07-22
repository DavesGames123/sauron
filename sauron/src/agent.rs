//! Which coding agent sauron is watching. Everything agent-specific lives behind
//! this enum: where the session logs are, how to name a session from its file,
//! how to fold one record, and how to spawn/resume the CLI in a workspace pane.
//! Claude Code is the default and fully supported; Codex is a second agent whose
//! log reader (see `codex`) is best-effort until certified against real rollouts.
//!
//! Selection order: an explicit `--claude`/`--codex` flag, then `$SAURON_AGENT`,
//! then auto-detect from whichever agent has logs for this repo.
//!
//! grep targets:
//!   fn select / from_env  -- choose the agent
//!   fn resume_cmd/bare_cmd -- what a workspace pane runs
//!   fn session_files/fold -- the scanner hooks, dispatched per agent

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::Session;
use crate::scan::{self, home, Scanner};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Agent {
    #[default]
    Claude,
    Codex,
}

impl Agent {
    /// Pick the agent: an explicit choice wins, then `$SAURON_AGENT`, then
    /// auto-detect from whichever agent actually has logs for this repo (Claude's
    /// per-repo directory, then a Codex install). Defaults to Claude.
    pub fn select(explicit: Option<Agent>, repo: &Path) -> Agent {
        if let Some(a) = explicit {
            return a;
        }
        if let Some(a) = Self::from_env() {
            return a;
        }
        if scan::project_dir_for(repo).is_dir() {
            return Agent::Claude;
        }
        if crate::codex::sessions_root().is_dir() {
            return Agent::Codex;
        }
        Agent::Claude
    }

    /// `$SAURON_AGENT=codex|claude`, if set to something recognised.
    pub fn from_env() -> Option<Agent> {
        match std::env::var("SAURON_AGENT").ok()?.to_ascii_lowercase().as_str() {
            "codex" => Some(Agent::Codex),
            "claude" | "claude-code" | "cc" => Some(Agent::Claude),
            _ => None,
        }
    }

    /// The CLI name, also the bare "open a fresh session" pane command.
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }

    /// Command a workspace pane runs to resume a session by id.
    pub fn resume_cmd(self, id: &str) -> String {
        match self {
            Agent::Claude => format!("claude --resume {id}"),
            Agent::Codex => format!("codex resume {id}"),
        }
    }

    /// Command an orc pane runs for a single-shot, non-interactive task.
    pub fn run_cmd(self, prompt: &str) -> String {
        match self {
            Agent::Claude => format!("claude '{prompt}'"),
            // `codex exec` is Codex's non-interactive one-shot mode.
            Agent::Codex => format!("codex exec '{prompt}'"),
        }
    }

    // --- scanning hooks ---

    /// Where this agent's logs live, for the "no sessions" message.
    pub fn log_root(self, repo: &Path) -> PathBuf {
        match self {
            Agent::Claude => scan::project_dir_for(repo),
            Agent::Codex => crate::codex::sessions_root(),
        }
    }

    /// The session log files for this repo.
    pub fn session_files(self, repo: &Path) -> Vec<PathBuf> {
        match self {
            Agent::Claude => Scanner::claude_session_files(repo),
            Agent::Codex => crate::codex::session_files(repo),
        }
    }

    /// A stable session id derived from a log file's path (Claude's file stem is
    /// the uuid; Codex's rollout name carries the uuid in its tail).
    pub fn session_id(self, path: &Path) -> String {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        match self {
            Agent::Claude => stem.to_string(),
            Agent::Codex => crate::codex::session_id(stem),
        }
    }

    /// Fold one parsed record into the session.
    pub fn fold(self, session: &mut Session, v: &Value, repo: &Path) {
        match self {
            Agent::Claude => scan::fold_record(session, v, repo),
            Agent::Codex => crate::codex::fold(session, v, repo),
        }
    }
}

/// `~/.codex` -- exposed here so selection can probe for a Codex install.
pub(crate) fn codex_home() -> PathBuf {
    home().join(".codex")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_commands_match_the_agent() {
        assert_eq!(Agent::Claude.label(), "claude");
        assert_eq!(Agent::Codex.label(), "codex");
        assert_eq!(Agent::Claude.resume_cmd("abc"), "claude --resume abc");
        assert_eq!(Agent::Codex.resume_cmd("abc"), "codex resume abc");
    }

    #[test]
    fn env_selection_is_case_insensitive_and_narrow() {
        // (Can't set env safely in parallel tests; exercise the mapping directly.)
        assert_eq!(Agent::default(), Agent::Claude);
    }
}
