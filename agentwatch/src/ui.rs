//! Rendering.
//!
//! Layout priority follows attention priority: sessions awaiting acknowledgement
//! get a banner, a colour, and full detail; sessions with nothing outstanding
//! collapse to a single count line. An earlier version gave every session three
//! lines regardless of state, so fifteen idle sessions buried the one that
//! needed a human.
//!
//! grep targets:
//!   fn draw            -- top-level layout
//!   fn header          -- repo name and the status tally
//!   fn section_header  -- coloured rule introducing each status group
//!   fn card            -- one session -> multi-line ListItem
//!   fn detail          -- selected session's write-set and prompt
//!   fn dim_common      -- shared directory prefix compression for path lists
//!   const AMBER/CYAN   -- the status palette

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::model::{ago, truncate, Status};
use crate::Row;

/// Status palette. Each state owns one hue and keeps it everywhere it appears --
/// glyph, title, section rule, header tally -- so the colour alone identifies
/// the state without reading the label.
const RED: Color = Color::Rgb(255, 92, 110); // blocked on your answer
const AMBER: Color = Color::Rgb(255, 176, 66); // awaiting acknowledgement
const CYAN: Color = Color::Rgb(86, 205, 226); // agent still working
const GREEN: Color = Color::Rgb(126, 200, 120); // nothing outstanding
const BLUE: Color = Color::Rgb(120, 170, 255); // chrome / repo identity
const DIM: Color = Color::Rgb(88, 94, 104);
const INK: Color = Color::Rgb(18, 20, 24); // text on a filled badge

pub fn color_of(status: Status) -> Color {
    match status {
        Status::Blocked => RED,
        Status::NeedsTest => AMBER,
        Status::Working => CYAN,
        Status::Clear => DIM,
    }
}

fn glyph_of(status: Status) -> &'static str {
    match status {
        Status::Blocked => "▲",
        Status::NeedsTest => "█",
        Status::Working => "◐",
        Status::Clear => "·",
    }
}

pub struct View<'a> {
    pub rows: &'a [Row],
    pub selected: usize,
    pub now: i64,
    pub repo: &'a str,
    pub saved: bool,
    pub hidden_stale: usize,
    pub clear_count: usize,
    pub show_clear: bool,
    pub copied: bool,
}

/// Screen geometry of the last-drawn list, so a mouse click can be resolved to
/// the row under it. Filled by `list`, read by the event loop.
#[derive(Default)]
pub struct FrameGeometry {
    pub list_top: u16,
    pub list_height: u16,
    /// Per rendered item, in draw order: its height in rows.
    pub item_heights: Vec<u16>,
    /// Per rendered item: the `rows` index it maps to, or None for a section
    /// header or the clear-collapse line.
    pub item_rows: Vec<Option<usize>>,
}

pub fn draw(f: &mut Frame, v: &View, list_state: &mut ListState, geo: &mut FrameGeometry) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(9),
            Constraint::Length(1),
        ])
        .split(f.area());

    header(f, chunks[0], v);
    list(f, chunks[1], v, list_state, geo);
    detail(f, chunks[2], v.rows.get(v.selected), v.now);
    footer(f, chunks[3], v);
    let _ = chunks;
}

fn header(f: &mut Frame, area: Rect, v: &View) {
    let awaiting = v.rows.iter().filter(|r| r.status == Status::NeedsTest).count();
    let working = v.rows.iter().filter(|r| r.status == Status::Working).count();

    let mut top = vec![
        Span::styled(
            format!(" {} ", v.repo),
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("agentwatch", Style::default().fg(DIM)),
        Span::raw("   "),
    ];

    // A blocked agent is doing nothing until you reply, so it gets the loudest
    // badge and sits leftmost -- ahead even of the awaiting count.
    let blocked = v.rows.iter().filter(|r| r.status == Status::Blocked).count();
    if blocked > 0 {
        top.push(Span::styled(
            format!(" {} WAITING ON YOU ", blocked),
            Style::default()
                .fg(INK)
                .bg(RED)
                .add_modifier(Modifier::BOLD),
        ));
        top.push(Span::raw("  "));
    }

    // The awaiting count is the whole reason the window is open, so it is a
    // filled badge rather than another line of coloured text.
    if awaiting > 0 {
        top.push(Span::styled(
            format!(" {} AWAITING YOU ", awaiting),
            Style::default()
                .fg(INK)
                .bg(AMBER)
                .add_modifier(Modifier::BOLD),
        ));
    } else if blocked == 0 {
        // Only claim this when nothing at all wants a human -- saying "all
        // caught up" beside a stalled agent would be a lie.
        top.push(Span::styled(
            " all caught up ",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ));
    }

    top.push(Span::raw("  "));
    if working > 0 {
        top.push(Span::styled(
            format!("◐ {} working", working),
            Style::default().fg(CYAN),
        ));
        top.push(Span::raw("  "));
    }
    top.push(Span::styled(
        format!("· {} clear", v.clear_count),
        Style::default().fg(DIM),
    ));

    let rule = Span::styled("─".repeat(area.width as usize), Style::default().fg(DIM));
    f.render_widget(
        Paragraph::new(vec![Line::from(top), Line::from(rule)]),
        area,
    );
}

fn list(f: &mut Frame, area: Rect, v: &View, list_state: &mut ListState, geo: &mut FrameGeometry) {
    let width = area.width.saturating_sub(1) as usize;

    geo.list_top = area.y;
    geo.list_height = area.height;
    geo.item_heights.clear();
    geo.item_rows.clear();

    if v.rows.is_empty() && v.clear_count == 0 {
        let empty = Paragraph::new("No sessions with repo edits yet.")
            .style(Style::default().fg(DIM));
        f.render_widget(empty, area);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_item = 0usize;
    let mut current: Option<Status> = None;

    // Push an item while recording its height and which row (if any) it maps to,
    // so a later mouse click can be resolved to the row under the cursor.
    let mut push = |items: &mut Vec<ListItem<'static>>, item: ListItem<'static>, row: Option<usize>| {
        geo.item_heights.push(item.height() as u16);
        geo.item_rows.push(row);
        items.push(item);
    };

    for (i, r) in v.rows.iter().enumerate() {
        if current != Some(r.status) {
            let n = v.rows.iter().filter(|x| x.status == r.status).count();
            push(&mut items, section_header(r.status, n, width), None);
            current = Some(r.status);
        }
        if i == v.selected {
            selected_item = items.len();
        }
        push(&mut items, card(r, i == v.selected, v.now, width), Some(i));
    }

    // Idle sessions carry no action, so they collapse to one line unless asked
    // for. This is the difference between a scannable window and a wall.
    if !v.show_clear && v.clear_count > 0 {
        let collapse = ListItem::new(vec![
            Line::raw(""),
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("· {} clear", v.clear_count),
                    Style::default().fg(DIM),
                ),
                Span::styled("  — press c to show", Style::default().fg(DIM)),
            ]),
        ]);
        push(&mut items, collapse, None);
    }

    list_state.select(if v.rows.is_empty() {
        None
    } else {
        Some(selected_item)
    });
    f.render_stateful_widget(List::new(items), area, list_state);
}

/// A coloured rule naming the group, so the eye lands on the boundary between
/// "needs me" and "does not" without reading any labels.
fn section_header(status: Status, count: usize, width: usize) -> ListItem<'static> {
    let (label, color) = match status {
        Status::Blocked => ("WAITING ON YOU", RED),
        Status::NeedsTest => ("AWAITING ACKNOWLEDGEMENT", AMBER),
        Status::Working => ("WORKING", CYAN),
        Status::Clear => ("CLEAR", DIM),
    };
    let text = format!(" {} ({}) ", label, count);
    let fill = width.saturating_sub(text.chars().count() + 1);

    ListItem::new(vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                text,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled("─".repeat(fill), Style::default().fg(DIM)),
        ]),
    ])
}

fn card(row: &Row, selected: bool, now: i64, width: usize) -> ListItem<'static> {
    let color = color_of(row.status);

    let marker = if selected {
        Span::styled("▎", Style::default().fg(color))
    } else {
        Span::raw(" ")
    };

    let title_style = match row.status {
        Status::Blocked => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        Status::NeedsTest => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        Status::Working => Style::default().fg(Color::Rgb(200, 210, 220)),
        Status::Clear => Style::default().fg(DIM),
    };

    let age = ago(row.last_activity, now);
    let name_room = width.saturating_sub(age.chars().count() + 12).max(16);

    let first = Line::from(vec![
        marker.clone(),
        Span::styled(format!("{} ", glyph_of(row.status)), Style::default().fg(color)),
        Span::styled(truncate(&row.name, name_room), title_style),
        Span::raw("  "),
        Span::styled(age, Style::default().fg(DIM)),
    ]);

    // A clear session has nothing to say beyond its name -- one line, no files.
    if row.status == Status::Clear {
        return ListItem::new(vec![first]);
    }

    let detail_color = if row.status == Status::NeedsTest {
        AMBER
    } else {
        DIM
    };
    // A blocked agent's file count is beside the point -- what it asked is what
    // you need. Show the prompt instead of "0 file(s) · all acked".
    let summary = if row.status == Status::Blocked {
        let why = row
            .blocked_reason
            .map(|r| r.short())
            .unwrap_or("waiting on you");
        match &row.last_prompt {
            Some(p) => format!(
                "{} · {}",
                why,
                truncate(
                    &crate::model::collapse_ws(p),
                    width.saturating_sub(why.len() + 10)
                )
            ),
            None => why.to_string(),
        }
    } else if row.pending.is_empty() {
        format!("{} file(s) · all acked", row.total_edits)
    } else {
        format!(
            "{} file(s) · {}",
            row.pending.len(),
            truncate(&dim_common(&row.pending), width.saturating_sub(22))
        )
    };

    let second = Line::from(vec![
        marker,
        Span::raw("   "),
        Span::styled(summary, Style::default().fg(detail_color)),
    ]);

    ListItem::new(vec![first, second, Line::raw("")])
}

fn detail(f: &mut Frame, area: Rect, row: Option<&Row>, now: i64) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(" detail ", Style::default().fg(DIM)));

    let Some(row) = row else {
        f.render_widget(
            Paragraph::new(Line::styled(
                "nothing selected",
                Style::default().fg(DIM),
            ))
            .block(block),
            area,
        );
        return;
    };

    let color = color_of(row.status);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", glyph_of(row.status)), Style::default().fg(color)),
        Span::styled(row.id_short.clone(), Style::default().fg(DIM)),
        Span::raw("  "),
        Span::styled(
            row.branch.clone().unwrap_or_else(|| "?".into()),
            Style::default().fg(BLUE),
        ),
        Span::raw("  "),
        Span::styled(
            format!("last activity {} ago", ago(row.last_activity, now)),
            Style::default().fg(DIM),
        ),
    ])];

    match row.status {
        Status::Blocked => {
            let text = match row.blocked_reason {
                Some(r) => format!("{} — switch to its terminal", r.detail()),
                None => "waiting on you".to_string(),
            };
            lines.push(Line::styled(
                text,
                Style::default().fg(RED).add_modifier(Modifier::BOLD),
            ));
        }
        Status::Working => lines.push(Line::styled(
            "agent is mid-turn — files still moving, do not test yet",
            Style::default().fg(CYAN),
        )),
        Status::Clear => lines.push(Line::styled(
            "nothing outstanding",
            Style::default().fg(GREEN),
        )),
        Status::NeedsTest => {
            lines.push(Line::styled(
                format!("{} untested write(s) — press a when checked", row.pending.len()),
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            ));
        }
    }

    // Past a handful the pane stops being scannable, which is the overhead this
    // tool exists to remove.
    for p in row.pending.iter().take(4) {
        lines.push(Line::from(vec![
            Span::styled("  › ", Style::default().fg(color)),
            Span::styled(p.clone(), Style::default().fg(Color::Rgb(210, 216, 224))),
        ]));
    }
    if row.pending.len() > 4 {
        lines.push(Line::styled(
            format!("    … and {} more", row.pending.len() - 4),
            Style::default().fg(DIM),
        ));
    }

    if let Some(prompt) = &row.last_prompt {
        lines.push(Line::from(vec![
            Span::styled("ask: ", Style::default().fg(DIM)),
            Span::styled(
                truncate(prompt, 200),
                Style::default().fg(Color::Rgb(150, 158, 170)),
            ),
        ]));
    }

    // The resume command, always present -- this is what lets a dropped thread
    // be picked back up. y (or a click) copies it.
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("continue ", Style::default().fg(DIM)),
        Span::styled("[y / click] ", Style::default().fg(BLUE)),
        Span::styled(
            row.continue_cmd.clone(),
            Style::default().fg(Color::Rgb(190, 200, 210)),
        ),
    ]));

    f.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

fn footer(f: &mut Frame, area: Rect, v: &View) {
    let key = |k: &'static str, what: &'static str| {
        vec![
            Span::styled(k, Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {}  ", what), Style::default().fg(DIM)),
        ]
    };
    let mut spans = vec![Span::raw(" ")];
    spans.extend(key("j/k", "move"));
    spans.extend(key("a", "ack"));
    spans.extend(key("u", "unack"));
    spans.extend(key("A", "ack all"));
    spans.extend(key("y", "copy continue"));
    spans.extend(key("c", if v.show_clear { "hide clear" } else { "show clear" }));
    spans.extend(key("q", "quit"));

    if v.hidden_stale > 0 {
        spans.push(Span::styled(
            format!("+{} older (o)  ", v.hidden_stale),
            Style::default().fg(DIM),
        ));
    }
    // The copied flash wins the corner when both fire -- it is the action the
    // user just took and wants confirmed.
    if v.copied {
        spans.push(Span::styled(
            "continue command copied",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ));
    } else if v.saved {
        spans.push(Span::styled(
            "saved",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Compress a path list to a shared prefix plus leaf names:
/// `src/gui/letters/{mod.rs, render.rs}`. Long lists are the norm here and the
/// shared prefix carries most of the meaning.
fn dim_common(paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    if paths.len() == 1 {
        return paths[0].clone();
    }

    let split: Vec<Vec<&str>> = paths.iter().map(|p| p.split('/').collect()).collect();
    let mut common = 0usize;
    'outer: loop {
        let Some(first) = split[0].get(common) else {
            break;
        };
        // Never consume the final component -- that would leave nothing to list.
        if common + 1 >= split[0].len() {
            break;
        }
        for s in &split {
            if s.get(common) != Some(first) || common + 1 >= s.len() {
                break 'outer;
            }
        }
        common += 1;
    }

    let prefix = split[0][..common].join("/");
    let leaves: Vec<String> = split.iter().map(|s| s[common..].join("/")).collect();

    if prefix.is_empty() {
        leaves.join(", ")
    } else {
        format!("{}/{{{}}}", prefix, leaves.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compresses_shared_directory_prefix() {
        let paths = vec![
            "src/gui/windows/letters/mod.rs".to_string(),
            "src/gui/windows/letters/render.rs".to_string(),
        ];
        assert_eq!(
            dim_common(&paths),
            "src/gui/windows/letters/{mod.rs, render.rs}"
        );
    }

    #[test]
    fn handles_disjoint_and_single_paths() {
        assert_eq!(dim_common(&["a.rs".to_string()]), "a.rs");
        let d = dim_common(&["src/a.rs".into(), "tools/b.rs".into()]);
        assert_eq!(d, "src/a.rs, tools/b.rs");
        assert_eq!(dim_common(&[]), "");
    }

    #[test]
    fn does_not_swallow_the_leaf_when_paths_nest() {
        let d = dim_common(&["src/a.rs".into(), "src/sub/b.rs".into()]);
        assert_eq!(d, "src/{a.rs, sub/b.rs}");
    }

    #[test]
    fn each_status_owns_a_distinct_colour() {
        // The palette is the primary signal; two states sharing a hue would make
        // the colour meaningless.
        assert_ne!(color_of(Status::NeedsTest), color_of(Status::Working));
        assert_ne!(color_of(Status::NeedsTest), color_of(Status::Clear));
        assert_ne!(color_of(Status::Working), color_of(Status::Clear));
    }
}
