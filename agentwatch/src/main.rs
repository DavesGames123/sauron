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

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEventKind,
};
use ratatui::crossterm::execute;
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
    /// `cd <cwd> && claude --resume <id>` -- reattach a dropped thread.
    pub continue_cmd: String,
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
    /// Transient "copied" banner deadline.
    copied_until: Option<Instant>,
    /// Per-frame list geometry, refreshed each draw, so a mouse click can be
    /// mapped back to the row under it. Heights and row-mapping run parallel to
    /// the items the list widget draws, in the same order.
    frame_item_heights: Vec<u16>,
    frame_item_rows: Vec<Option<usize>>,
    list_top: u16,
    list_height: u16,
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
            copied_until: None,
            frame_item_heights: Vec::new(),
            frame_item_rows: Vec::new(),
            list_top: 0,
            list_height: 0,
        };
        app.refresh();
        app.focus_first_actionable();
        app
    }

    /// On launch, land the cursor on the thing most likely to want the user:
    /// a session waiting on them, else one actively working, else untested work.
    /// Runs once at startup; later refreshes preserve the selection by id, so the
    /// cursor is not yanked around as statuses churn.
    fn focus_first_actionable(&mut self) {
        let order = [Status::Blocked, Status::Working, Status::NeedsTest];
        for want in order {
            if let Some(i) = self.rows.iter().position(|r| r.status == want) {
                self.selected = i;
                self.sync_list_state();
                return;
            }
        }
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
                // Within the blocked band, a certain question outranks a guessed
                // approval outranks a plain idle-stop (BlockedReason is ordered).
                .then(a.blocked_reason.cmp(&b.blocked_reason))
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
        let blocked_reason = s.blocked_reason(now, acked);
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
            continue_cmd: s.continue_command(),
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

    fn copied_flash(&self) -> bool {
        self.copied_until.map(|t| Instant::now() < t).unwrap_or(false)
    }

    /// Copy the selected session's resume command to the clipboard.
    fn copy_selected_continue(&mut self) {
        self.copy_continue_for(self.selected);
    }

    fn copy_continue_for(&mut self, idx: usize) {
        let Some(row) = self.rows.get(idx) else {
            return;
        };
        if copy_to_clipboard(&row.continue_cmd) {
            self.selected = idx;
            self.sync_list_state();
            self.copied_until = Some(Instant::now() + Duration::from_millis(1600));
        }
    }

    /// Map a terminal cell to the row rendered under it, using the geometry
    /// captured on the last frame.
    fn row_at(&self, click_y: u16) -> Option<usize> {
        hit_test(
            click_y,
            self.list_top,
            self.list_height,
            self.list_state.offset(),
            &self.frame_item_heights,
            &self.frame_item_rows,
        )
    }
}

/// Resolve a click's y-coordinate to a row index, walking the visible items from
/// the scroll offset and summing their heights. Returns None for a click on a
/// section header, the clear-collapse line, or empty space below the list.
/// Pure arithmetic, split out from `App` so it can be tested without a terminal.
fn hit_test(
    click_y: u16,
    list_top: u16,
    list_height: u16,
    offset: usize,
    heights: &[u16],
    rows: &[Option<usize>],
) -> Option<usize> {
    if click_y < list_top || click_y >= list_top + list_height {
        return None;
    }
    let mut y = list_top;
    for i in offset..heights.len() {
        let h = heights[i];
        if click_y >= y && click_y < y + h {
            return rows.get(i).copied().flatten();
        }
        y += h;
        if y >= list_top + list_height {
            break;
        }
    }
    None
}

/// Put text on the system clipboard. Tries pbcopy first (macOS), then falls
/// back to an OSC 52 escape so it still works over SSH or on a bare terminal.
fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            return true;
        }
    }

    // OSC 52: ESC ] 52 ; c ; <base64> BEL -- ask the terminal to set the
    // clipboard. Written straight to the tty; harmless if unsupported.
    let seq = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    std::io::stdout().write_all(seq.as_bytes()).is_ok()
        && std::io::stdout().flush().is_ok()
}

/// Minimal standard-alphabet base64, to avoid a dependency for one escape code.
fn base64(input: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(A[(n >> 18 & 63) as usize] as char);
        out.push(A[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { A[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { A[(n & 63) as usize] as char } else { '=' });
    }
    out
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
    // Mouse capture lets a click copy a session's continue command. The cost is
    // that click-drag no longer selects terminal text -- hold Option (iTerm2) or
    // Shift to select while this runs.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let mut last_tick = Instant::now();

    let result = loop {
        let now = now_ms();
        let rows = app.rows.clone();
        let selected = app.selected;
        let label = app.repo_label.clone();
        let saved = app.saved_flash();
        let copied = app.copied_flash();
        let mut list_state = app.list_state.clone();
        let mut geo = ui::FrameGeometry::default();

        let view = ui::View {
            rows: &rows,
            selected,
            now,
            repo: &label,
            saved,
            hidden_stale: app.hidden_stale,
            clear_count: app.clear_count,
            show_clear: app.show_clear,
            copied,
        };
        if let Err(e) = terminal.draw(|f| ui::draw(f, &view, &mut list_state, &mut geo)) {
            break Err(e);
        }
        app.list_state = list_state;
        // Stash the frame's list geometry so a mouse click next iteration can be
        // mapped to the row under the cursor.
        app.frame_item_heights = geo.item_heights;
        app.frame_item_rows = geo.item_rows;
        app.list_top = geo.list_top;
        app.list_height = geo.list_height;

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
                    KeyCode::Char('y') => app.copy_selected_continue(),
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
                Ok(Event::Mouse(m)) => match m.kind {
                    // Click a row to copy its continue command (and select it).
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(idx) = app.row_at(m.row) {
                            app.copy_continue_for(idx);
                        }
                    }
                    MouseEventKind::ScrollDown => app.move_by(1),
                    MouseEventKind::ScrollUp => app.move_by(-1),
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

    let _ = execute!(std::io::stdout(), DisableMouseCapture);
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
    let blocked = app.rows.iter().filter(|r| r.status == Status::Blocked).count();
    let needs = app.rows.iter().filter(|r| r.status == Status::NeedsTest).count();
    let mut banner = Vec::new();
    if blocked > 0 {
        banner.push(format!("{} WAITING ON YOU", blocked));
    }
    if needs > 0 {
        banner.push(format!("{} to test", needs));
    }
    println!("{}\n", if banner.is_empty() { "all caught up".into() } else { banner.join("  ·  ") });

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
        // Blocked sessions: what they want matters more than a file count.
        if r.status == Status::Blocked {
            if let Some(reason) = r.blocked_reason {
                println!("    {}", reason.detail());
            }
        } else if r.pending.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    // Layout used by these tests: list starts at y=10, is 20 rows tall.
    // Items (draw order): section header (h=2), card A (h=3), card B (h=3),
    // section header (h=2), card C (h=3). Rows map: A->0, B->1, C->2.
    fn fixture() -> (Vec<u16>, Vec<Option<usize>>) {
        (
            vec![2, 3, 3, 2, 3],
            vec![None, Some(0), Some(1), None, Some(2)],
        )
    }

    #[test]
    fn click_lands_on_the_card_under_the_cursor() {
        let (h, r) = fixture();
        // y layout from top=10: header 10-11, A 12-14, B 15-17, header 18-19, C 20-22.
        assert_eq!(hit_test(13, 10, 20, 0, &h, &r), Some(0)); // inside card A
        assert_eq!(hit_test(16, 10, 20, 0, &h, &r), Some(1)); // inside card B
        assert_eq!(hit_test(21, 10, 20, 0, &h, &r), Some(2)); // inside card C
    }

    #[test]
    fn clicks_on_headers_and_outside_resolve_to_nothing() {
        let (h, r) = fixture();
        assert_eq!(hit_test(10, 10, 20, 0, &h, &r), None); // section header
        assert_eq!(hit_test(18, 10, 20, 0, &h, &r), None); // second header
        assert_eq!(hit_test(5, 10, 20, 0, &h, &r), None); // above the list
        assert_eq!(hit_test(40, 10, 20, 0, &h, &r), None); // below the list
    }

    #[test]
    fn scroll_offset_shifts_the_hit_test() {
        let (h, r) = fixture();
        // With offset=1, the first drawn item is card A at the list top (y=10).
        // header 10-11? no -- offset skips item 0, so A(h=3) is 10-12, B 13-15...
        assert_eq!(hit_test(11, 10, 20, 1, &h, &r), Some(0));
        assert_eq!(hit_test(14, 10, 20, 1, &h, &r), Some(1));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
