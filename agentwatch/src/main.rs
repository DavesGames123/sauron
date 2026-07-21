//! agentwatch -- a sidecar that answers one question: what did my agents change
//! that I have not tested yet?
//!
//! Everything it shows is read out of the Claude Code session logs at
//! ~/.claude/projects/<encoded-repo-path>/*.jsonl. It writes nothing to the repo
//! and never talks to a running agent.
//!
//! Usage:
//!   agentwatch                  # watch the repo containing the cwd
//!   agentwatch /path/to/repo    # watch a specific repo
//!
//! grep targets:
//!   struct Row          -- one session flattened for rendering
//!   struct App          -- scanner + ack store + selection
//!   fn App::refresh     -- rescan logs and rebuild rows
//!   fn main             -- terminal lifecycle and event loop

mod model;
mod scan;
mod store;
mod ui;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::widgets::ListState;

use model::{now_ms, BlockedReason, Session, Status, DORMANT_AFTER_MS, STALE_HORIZON_MS};
use scan::Scanner;
use store::AckStore;

/// How often the logs are re-tailed. Only appended bytes are parsed, so this is
/// cheap even with 10MB session files.
const TICK: Duration = Duration::from_millis(2000);

#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub id_short: String,
    pub name: String,
    pub branch: Option<String>,
    pub last_activity: i64,
    pub status: Status,
    pub blocked_reason: Option<BlockedReason>,
    /// Repo paths written but not acked at their current timestamp.
    pub pending: Vec<String>,
    pub total_edits: usize,
    pub last_prompt: Option<String>,
    pub edits: BTreeMap<String, i64>,
}

struct App {
    scanner: Scanner,
    store: AckStore,
    /// Rows past the staleness horizon, kept out of `rows` but counted.
    rows: Vec<Row>,
    hidden_stale: usize,
    /// Clear sessions are counted but kept out of `rows` unless `show_clear`.
    clear_count: usize,
    show_clear: bool,
    show_all: bool,
    list_state: ListState,
    selected: usize,
    repo_label: String,
    saved_until: Option<Instant>,
}

impl App {
    fn new(repo_root: PathBuf) -> Self {
        let repo_label = repo_root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo_root.to_string_lossy().into_owned());

        let mut app = Self {
            scanner: Scanner::new(repo_root),
            store: AckStore::load(),
            rows: Vec::new(),
            hidden_stale: 0,
            clear_count: 0,
            show_clear: false,
            show_all: false,
            list_state: ListState::default(),
            selected: 0,
            repo_label,
            saved_until: None,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let now = now_ms();
        let sessions = self.scanner.refresh();

        // Preserve the selected session across rebuilds -- rows reorder as
        // statuses change, and moving the cursor out from under the user is the
        // fastest way to make them ack the wrong thing.
        let anchor = self.rows.get(self.selected).map(|r| r.id.clone());

        let mut rows: Vec<Row> = sessions
            .into_iter()
            .filter_map(|s| self.to_row(s, now))
            .collect();

        rows.sort_by(|a, b| {
            a.status
                .rank()
                .cmp(&b.status.rank())
                .then(b.last_activity.cmp(&a.last_activity))
        });

        // Collapse the historical backlog. Sessions from days ago were tested
        // (or abandoned) by whatever process preceded this tool; listing them as
        // outstanding buries today's actual work.
        let before = rows.len();
        if !self.show_all {
            rows.retain(|r| {
                r.status != Status::NeedsTest
                    || now.saturating_sub(r.last_activity) <= STALE_HORIZON_MS
            });
        }
        self.hidden_stale = before - rows.len();

        // Idle sessions carry no action. Counting them is useful; giving each
        // one three lines of the window is not.
        self.clear_count = rows.iter().filter(|r| r.status == Status::Clear).count();
        if !self.show_clear {
            rows.retain(|r| r.status != Status::Clear);
        }

        self.rows = rows;
        self.selected = anchor
            .and_then(|id| self.rows.iter().position(|r| r.id == id))
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        self.sync_list_state();
    }

    fn to_row(&self, s: Session, now: i64) -> Option<Row> {
        let acked = self.store.for_session(&s.id);
        let status = s.status(now, acked);
        let blocked_reason = s.blocked_reason(now);
        let pending: Vec<String> = s.pending(acked, now).into_iter().map(String::from).collect();

        // A session with no repo edits still matters while it is live -- it is
        // holding an agent slot, and if it is blocked on a question the delay is
        // yours to clear. Only drop it once it has gone quiet with nothing to
        // show, which is what a finished chat-only session looks like.
        if s.edits.is_empty() && status == Status::Clear {
            return None;
        }
        if status == Status::Clear && now.saturating_sub(s.last_activity) > DORMANT_AFTER_MS {
            return None;
        }

        Some(Row {
            id: s.id.clone(),
            id_short: s.short_id().to_string(),
            name: s.display_name(),
            branch: s.branch.clone(),
            last_activity: s.last_activity,
            status,
            blocked_reason,
            pending,
            total_edits: s.edits.len(),
            last_prompt: s.last_prompt.clone(),
            edits: s.edits,
        })
    }

    fn sync_list_state(&mut self) {
        if self.rows.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(self.selected));
        }
    }

    fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
        self.sync_list_state();
    }

    fn ack_selected(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let (id, edits) = (row.id.clone(), row.edits.clone());
        self.store.ack(&id, &edits);
        self.persist();
        self.refresh();
    }

    fn unack_selected(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let id = row.id.clone();
        self.store.unack(&id);
        self.persist();
        self.refresh();
    }

    /// Ack every outstanding session, including ones hidden by the horizon.
    /// This is the cold-start move: declare the historical backlog tested so the
    /// queue starts empty and only new agent work appears.
    fn baseline(&mut self) {
        let saved = self.show_all;
        self.show_all = true;
        self.refresh();
        self.ack_all();
        self.show_all = saved;
        self.refresh();
    }

    fn ack_all(&mut self) {
        let all: Vec<(String, BTreeMap<String, i64>)> = self
            .rows
            .iter()
            .filter(|r| r.status == Status::NeedsTest)
            .map(|r| (r.id.clone(), r.edits.clone()))
            .collect();
        for (id, edits) in all {
            self.store.ack(&id, &edits);
        }
        self.persist();
        self.refresh();
    }

    fn persist(&mut self) {
        if self.store.save().is_ok() {
            self.saved_until = Some(Instant::now() + Duration::from_millis(1200));
        }
    }

    fn store_len(&self) -> usize {
        self.store.session_count()
    }

    fn saved_flash(&self) -> bool {
        self.saved_until.map(|t| Instant::now() < t).unwrap_or(false)
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let once = args.iter().any(|a| a == "--once");
    let baseline = args.iter().any(|a| a == "--baseline");
    let list_working = args.iter().any(|a| a == "--list-working");
    let repo_root = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => PathBuf::from(p),
        None => git_root().unwrap_or(std::env::current_dir()?),
    };
    let repo_root = repo_root.canonicalize().unwrap_or(repo_root);

    let mut app = App::new(repo_root);

    if !app.scanner.log_dir().exists() {
        eprintln!(
            "no Claude Code session logs for {}\n  looked in: {}",
            app.scanner.repo_root().display(),
            app.scanner.log_dir().display()
        );
        return Ok(());
    }

    if baseline {
        app.baseline();
        println!(
            "baselined: {} session(s) marked tested. Only new agent work will appear from here.",
            app.store_len()
        );
        return Ok(());
    }

    // Headless list of the sessions agentwatch currently counts as Working, one
    // per line as `session_id<TAB>display_name`. Consumed by the `workspace`
    // tool to reopen each in-flight agent (`claude --resume <id>`) into a pane.
    // Working = mid-turn and not stuck, i.e. the same "◐ working" set the TUI
    // shows -- reusing App::rows means the definition can never drift from it.
    if list_working {
        for r in &app.rows {
            if r.status == Status::Working {
                println!("{}\t{}", r.id, model::collapse_ws(&r.name));
            }
        }
        return Ok(());
    }

    // Snapshot mode: print and exit without touching the terminal. Also the only
    // way to validate the scanner against real logs without an interactive TTY.
    if once {
        print_once(&app);
        return Ok(());
    }

    let mut terminal = ratatui::init();
    let mut last_tick = Instant::now();

    let result = loop {
        let now = now_ms();
        let rows = app.rows.clone();
        let selected = app.selected;
        let label = app.repo_label.clone();
        let saved = app.saved_flash();
        let mut list_state = app.list_state.clone();

        let view = ui::View {
            rows: &rows,
            selected,
            now,
            repo: &label,
            saved,
            hidden_stale: app.hidden_stale,
            clear_count: app.clear_count,
            show_clear: app.show_clear,
        };
        if let Err(e) = terminal.draw(|f| ui::draw(f, &view, &mut list_state)) {
            break Err(e);
        }
        app.list_state = list_state;

        // Poll rather than block so the tick still fires on an idle keyboard.
        let timeout = TICK.saturating_sub(last_tick.elapsed());
        match event::poll(timeout) {
            Ok(true) => match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char('j') | KeyCode::Down => app.move_by(1),
                    KeyCode::Char('k') | KeyCode::Up => app.move_by(-1),
                    KeyCode::Char('a') => app.ack_selected(),
                    KeyCode::Char('u') => app.unack_selected(),
                    KeyCode::Char('A') => app.ack_all(),
                    KeyCode::Char('o') => {
                        app.show_all = !app.show_all;
                        app.refresh();
                    }
                    KeyCode::Char('c') => {
                        app.show_clear = !app.show_clear;
                        app.refresh();
                    }
                    KeyCode::Char('r') => app.refresh(),
                    _ => {}
                },
                Ok(_) => {}
                Err(e) => break Err(e),
            },
            Ok(false) => {}
            Err(e) => break Err(e),
        }

        if last_tick.elapsed() >= TICK {
            app.refresh();
            last_tick = Instant::now();
        }
    };

    ratatui::restore();
    result
}

/// Plain-text snapshot for `--once`.
fn print_once(app: &App) {
    let now = now_ms();
    if app.rows.is_empty() {
        println!("no sessions with repo edits");
        return;
    }
    let awaiting = app
        .rows
        .iter()
        .filter(|r| r.status == Status::NeedsTest)
        .count();
    if awaiting > 0 {
        println!("{} AWAITING YOU\n", awaiting);
    } else {
        println!("all caught up\n");
    }

    for r in &app.rows {
        let glyph = match r.status {
            Status::Blocked => "▲",
            Status::NeedsTest => "█",
            Status::Working => "◐",
            Status::Clear => "·",
        };
        println!(
            "{} {:<52} {:>4}  {}",
            glyph,
            model::truncate(&r.name, 52),
            model::ago(r.last_activity, now),
            r.status.label()
        );
        if r.pending.is_empty() {
            println!("    {} file(s), all acked", r.total_edits);
        } else {
            for p in r.pending.iter().take(6) {
                println!("    {}", p);
            }
            if r.pending.len() > 6 {
                println!("    … and {} more", r.pending.len() - 6);
            }
        }
        println!();
    }
}

/// Walk up from the cwd looking for a .git entry, so the tool works from any
/// subdirectory of the repo.
fn git_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}
