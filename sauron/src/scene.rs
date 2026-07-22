//! The five-line animated Eye of Sauron that crowns the header when the terminal
//! is tall enough. Everything here is a pure function of elapsed milliseconds, so
//! nothing has to be ticked or stored between frames and the whole thing is
//! testable off the clock alone.
//!
//! What it shows:
//!   - the lidless Eye (block art) atop its tower, at the right: it glances,
//!     blinks, and flickers while idle;
//!   - an engraving of Sindarin in runic script (real Unicode runes, U+16A0,
//!     which render out of the box -- true Tengwar lives in the private-use area
//!     and needs a font, so this stands in for it) with a faint gloss;
//!   - and, now and then, Frodo and Sam scurrying along the ground with Gollum
//!     creeping behind and the Ring glinting -- while the Eye's pupil tracks them
//!     and flares wide as they pass beneath it.
//!
//! grep targets:
//!   fn scene           -- compose the five lines for a given width and ms
//!   fn eye_rows        -- the Eye sprite for a pose + pupil column
//!   fn walker_sprites  -- Frodo / Sam / Gollum, facing left or right
//!   fn runic           -- latin -> Elder Futhark transliteration
//!   const VERSES       -- the engraved Sindarin lines
//!   enum Pose / fn idle_pose / walk timing consts

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::{DIM, EMBER, FLAME, FLARE, RUNE};

/// Rows the scene occupies (not counting the status line above it).
pub const HEIGHT: u16 = 5;

const RING: Color = Color::Rgb(255, 214, 96); // the One Ring's glint
const HOBBIT: Color = Color::Rgb(150, 190, 140); // Frodo & Sam
const GOLLUM_C: Color = Color::Rgb(150, 172, 150); // the pale creeping thing
const GROUND: Color = Color::Rgb(66, 70, 80); // the horizon the walkers cross
const STONE: Color = Color::Rgb(92, 96, 112); // the black tower of Barad-dûr
const HOT: Color = Color::Rgb(255, 236, 150); // white-hot: flame tips and the Eye's core
const RED: Color = Color::Rgb(200, 54, 20); // deep red: the cool outer edge of the fire

// Timeline: one walk per 26s, crossing over ~7.5s, a long calm idle the rest.
const PERIOD: u64 = 26_000;
const WALK_START: u64 = 17_000;
const WALK_END: u64 = 24_500;

// Width of the Eye sprite in cells, and how far its left edge sits from the
// right margin. Every glyph used is unambiguous-width-1 (block, box-drawing,
// runic, latin, ascii), so a char index equals a screen column.
const EYE_W: usize = 13;
const EYE_MARGIN: usize = 15;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Pose {
    Center,
    Blink,
    Wide,
}

type Cell = (char, Style);

fn blank_row(w: usize) -> Vec<Cell> {
    vec![(' ', Style::default()); w]
}

/// Overlay `s` at column `x`. Spaces in the sprite are transparent, so sprites
/// can be layered without clobbering what is behind their padding.
fn stamp(row: &mut [Cell], x: i32, s: &str, st: Style) {
    for (i, ch) in s.chars().enumerate() {
        if ch == ' ' {
            continue;
        }
        let c = x + i as i32;
        if c >= 0 && (c as usize) < row.len() {
            row[c as usize] = (ch, st);
        }
    }
}

/// Collapse a styled cell row into spans, merging runs of equal style so a line
/// is a handful of spans rather than one per cell.
fn row_to_line(row: Vec<Cell>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur = String::new();
    let mut cur_st: Option<Style> = None;
    for (ch, st) in row {
        if cur_st != Some(st) {
            if let Some(s) = cur_st {
                spans.push(Span::styled(std::mem::take(&mut cur), s));
            }
            cur_st = Some(st);
        }
        cur.push(ch);
    }
    if let Some(s) = cur_st {
        spans.push(Span::styled(cur, s));
    }
    Line::from(spans)
}

/// The Eye set atop Barad-dûr: five rows, `EYE_W` wide. The Eye burns -- an
/// animated flame crown with white-hot tips and sparks, and a hot -> orange ->
/// deep-red gradient across the fire so it glows like an ember rather than a
/// flat block -- all framed by the stone tower. Pupil column is 0..=6.
fn eye_tower(pose: Pose, pupil: usize, flicker: usize) -> [Vec<Cell>; 5] {
    let bright = pose == Pose::Wide; // when it flares, every band shifts hotter
    let stone = Style::default().fg(STONE);
    let dark = Style::default().fg(EMBER);
    let lid = Style::default().fg(RUNE);
    let hot = Style::default().fg(HOT);
    let flare = Style::default().fg(if bright { HOT } else { FLARE });
    let flame = Style::default().fg(if bright { FLARE } else { FLAME });
    let red = Style::default().fg(if bright { FLAME } else { RED });

    // The flame crown: three frames of licking fire with sparks flying off the
    // edges. Tips and sparks are white-hot; the body glows.
    let crowns = ["  ▖▄▟███▙▄▗  ", "  ▗▟█▟█▙█▙▖  ", "  ▘▄▟█▙▟█▄▝  "];
    let mut r0 = blank_row(EYE_W);
    for (i, ch) in crowns[flicker % crowns.len()].chars().enumerate() {
        if ch == ' ' {
            continue;
        }
        // Sparks (quadrant dots) and flame tips (▄▀) are hottest.
        let st = match ch {
            '▖' | '▗' | '▘' | '▝' | '▄' | '▀' => hot,
            _ => flare,
        };
        r0[i] = (ch, st);
    }

    // Eye upper/lower: red-hot outer corners fading to orange across the middle.
    let mut r1 = blank_row(EYE_W);
    r1[1] = ('█', stone);
    r1[2] = ('▟', red);
    for c in 3..10 {
        r1[c] = ('█', flame);
    }
    r1[10] = ('▙', red);
    r1[11] = ('█', stone);
    let mut r3 = blank_row(EYE_W);
    r3[1] = ('█', stone);
    r3[2] = ('▜', red);
    for c in 3..10 {
        r3[c] = ('█', flame);
    }
    r3[10] = ('▛', red);
    r3[11] = ('█', stone);

    // The Eye's middle -- the hottest band. Iris (cols 3..=9) runs a gradient
    // from a white-hot core out to red edges; the pupil is a dark hole in it.
    let mut r2 = blank_row(EYE_W);
    r2[1] = ('█', stone);
    r2[11] = ('█', stone);
    r2[2] = ('▐', red);
    r2[10] = ('▌', red);
    match pose {
        Pose::Blink => {
            for c in 3..10 {
                r2[c] = ('━', lid);
            }
        }
        _ => {
            for i in 0..7usize {
                let col = 3 + i;
                let heat = match (col as i32 - 6).abs() {
                    0 => hot,
                    1 => flare,
                    2 => flame,
                    _ => red,
                };
                let is_pupil = if pose == Pose::Wide {
                    (2..=4).contains(&i)
                } else {
                    i == pupil.min(6)
                };
                r2[col] = ('█', if is_pupil { dark } else { heat });
            }
        }
    }

    // The tower foot, flaring out where it meets the ground.
    let mut r4 = blank_row(EYE_W);
    for (i, ch) in "▟███████████▙".chars().enumerate() {
        r4[i] = (ch, stone);
    }

    [r0, r1, r2, r3, r4]
}

/// Frodo, Sam, Gollum as two-cell sprites, mid-stride, facing right or left.
fn walker_sprites(leg: usize, right: bool) -> (&'static str, &'static str, &'static str) {
    match (leg % 2, right) {
        (0, true) => ("ó╱", "ô╱", "o╮"),
        (1, true) => ("ó╲", "ô╲", "o╯"),
        (0, false) => ("╲ó", "╲ô", "╭o"),
        _ => ("╱ó", "╱ô", "╰o"),
    }
}

/// The idle Eye: a long, level, watchful stare with only rare, slow motion -- a
/// couple of blinks and one held glance across the whole idle stretch, so it
/// broods rather than darts about. `t` is the phase within `PERIOD`; the walk
/// owns 17s..24.5s, so the events here sit in the calm before it.
fn idle_pose(t: u64) -> (Pose, usize) {
    match t {
        5_000..=5_250 => (Pose::Blink, 3),
        10_500..=11_599 => (Pose::Center, 0), // a slow glance left, held
        11_600..=11_850 => (Pose::Blink, 3),  // and a blink as it returns
        _ => (Pose::Center, 3),               // the level stare (centre of 7)
    }
}

/// Whether a walk is happening at phase `t`, and if so its progress 0..1 and
/// whether it heads rightward. Kept separate so the schedule is unit-testable.
fn walk_at(t: u64, cyc: u64) -> Option<(f64, bool)> {
    if (WALK_START..WALK_END).contains(&t) {
        let prog = (t - WALK_START) as f64 / (WALK_END - WALK_START) as f64;
        Some((prog, cyc.is_multiple_of(2)))
    } else {
        None
    }
}

/// The engraved lines: canonical Sindarin (the West-gate of Moria), shown in
/// runes with a faint English gloss. Real Elvish words, real script glyphs.
const VERSES: [(&str, &str); 3] = [
    ("pedo mellon a minno", "speak, friend, and enter"),
    ("ennyn durin aran moria", "the doors of durin, lord of moria"),
    ("celebrimbor teithant", "celebrimbor drew these signs"),
];

fn verse(ms: u64) -> (&'static str, &'static str) {
    // A slow engraving: each line lingers for half a minute.
    VERSES[((ms / 30_000) % VERSES.len() as u64) as usize]
}

/// Latin -> Elder Futhark. `th` becomes the thorn rune; unknown chars pass
/// through so punctuation and the like survive.
fn runic(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == 't' && chars[i + 1] == 'h' {
            out.push('ᚦ');
            i += 2;
            continue;
        }
        out.push(rune(chars[i]));
        i += 1;
    }
    out
}

fn rune(c: char) -> char {
    match c.to_ascii_lowercase() {
        'a' => 'ᚨ', 'b' => 'ᛒ', 'c' => 'ᚲ', 'd' => 'ᛞ', 'e' => 'ᛖ', 'f' => 'ᚠ',
        'g' => 'ᚷ', 'h' => 'ᚺ', 'i' => 'ᛁ', 'j' => 'ᛃ', 'k' => 'ᚲ', 'l' => 'ᛚ',
        'm' => 'ᛗ', 'n' => 'ᚾ', 'o' => 'ᛟ', 'p' => 'ᛈ', 'q' => 'ᚲ', 'r' => 'ᚱ',
        's' => 'ᛊ', 't' => 'ᛏ', 'u' => 'ᚢ', 'v' => 'ᚹ', 'w' => 'ᚹ', 'x' => 'ᛉ',
        'y' => 'ᛃ', 'z' => 'ᛉ', ' ' => '᛬', other => other,
    }
}

/// Truncate a string to `max` characters (runes are single width, so char count
/// is column count).
fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Compose the five scene lines for a terminal `width` at time `ms`.
pub fn scene(width: usize, ms: u64) -> Vec<Line<'static>> {
    let w = width.max(20);
    let mut grid: Vec<Vec<Cell>> = (0..5).map(|_| blank_row(w)).collect();

    // The ground.
    let ground_style = Style::default().fg(GROUND);
    for cell in grid[4].iter_mut() {
        *cell = ('▁', ground_style);
    }

    let t = ms % PERIOD;
    let cyc = ms / PERIOD;
    let leg = ((ms / 180) % 2) as usize;
    let flicker = ((ms / 220) % 3) as usize; // the fire licks; the pupil stays calm
    let eye_left = w.saturating_sub(EYE_MARGIN) as i32;
    let eye_col = eye_left + 6; // the pupil's screen column

    // Resolve the Eye's pose, and remember the procession (if any) so it can be
    // drawn in front of the tower foot rather than behind it.
    let mut walk: Option<(i32, bool)> = None;
    let (pose, pupil) = if let Some((prog, right)) = walk_at(t, cyc) {
        let span = (w + 16) as f64;
        let base = if right {
            (-12.0 + prog * span).floor() as i32
        } else {
            (w as f64 + 4.0 - prog * span).floor() as i32
        };
        walk = Some((base, right));
        // The Eye follows the lead hobbit and flares wide as they pass beneath.
        let frac = (base as f64 / w as f64).clamp(0.0, 1.0);
        let pupil = (frac * 6.0).round() as usize;
        let pose = if (eye_col - base).abs() < 4 {
            Pose::Wide
        } else {
            Pose::Center
        };
        (pose, pupil.min(6))
    } else {
        idle_pose(t)
    };

    // The tower and its Eye occupy all five rows; the foot lands on the ground.
    for (r, row) in eye_tower(pose, pupil, flicker).into_iter().enumerate() {
        for (i, (ch, st)) in row.into_iter().enumerate() {
            if ch == ' ' {
                continue;
            }
            let c = eye_left + i as i32;
            if c >= 0 && (c as usize) < w {
                grid[r][c as usize] = (ch, st);
            }
        }
    }

    // The procession, in the foreground, so the hobbits pass in front of the
    // tower's foot instead of vanishing behind it.
    if let Some((base, right)) = walk {
        let (fr, sm, go) = walker_sprites(leg, right);
        let hob = Style::default().fg(HOBBIT);
        let gol = Style::default().fg(GOLLUM_C);
        let ring = Style::default().fg(RING).add_modifier(Modifier::BOLD);
        if right {
            // Frodo leads, Sam a step back, Gollum skulking further behind, the
            // Ring glinting just ahead of Frodo.
            stamp(&mut grid[4], base, fr, hob);
            stamp(&mut grid[4], base - 3, sm, hob);
            stamp(&mut grid[4], base - 9, go, gol);
            stamp(&mut grid[4], base + 2, "*", ring);
        } else {
            stamp(&mut grid[4], base, fr, hob);
            stamp(&mut grid[4], base + 3, sm, hob);
            stamp(&mut grid[4], base + 9, go, gol);
            stamp(&mut grid[4], base - 2, "*", ring);
        }
    }

    // The engraving, top-left: runic script on row 1, faint gloss on row 0. Clip
    // both so they never run into the Eye on a narrow terminal.
    let (words, gloss) = verse(ms);
    let room = eye_left.max(2) as usize - 2;
    stamp(&mut grid[1], 1, &clip(&runic(words), room), Style::default().fg(RUNE));
    stamp(&mut grid[0], 1, &clip(gloss, room), Style::default().fg(DIM));

    grid.into_iter().map(row_to_line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_width(l: &Line) -> usize {
        l.spans.iter().map(|s| s.content.chars().count()).sum()
    }
    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn scene_is_five_lines_and_never_overflows_width() {
        for &w in &[24usize, 40, 80, 120] {
            for ms in [0u64, 6_000, 20_000, 21_000, 23_000, 46_000, 999_999] {
                let s = scene(w, ms);
                assert_eq!(s.len(), 5, "w={w} ms={ms}");
                for l in &s {
                    assert!(line_width(l) <= w, "overflow at w={w} ms={ms}: {}", line_text(l));
                }
            }
        }
    }

    #[test]
    fn walkers_appear_only_during_a_walk_window() {
        // Mid-idle: no hobbit on the ground.
        let idle = line_text(&scene(80, 6_000)[4]);
        assert!(!idle.contains('ó'), "hobbit showed up while idle: {idle}");
        // Mid-walk (the window is 17s..24.5s): Frodo, Sam, and the Ring cross.
        let walk = line_text(&scene(80, 20_000)[4]);
        assert!(walk.contains('ó'), "Frodo missing mid-walk: {walk}");
        assert!(walk.contains('ô'), "Sam missing mid-walk: {walk}");
        assert!(walk.contains('*'), "the Ring's glint is missing: {walk}");
    }

    #[test]
    fn eye_blinks_on_the_idle_schedule() {
        // The idle blink sits at t≈5000.
        assert_eq!(idle_pose(5_100).0, Pose::Blink);
        // A blink renders the lid rune on the Eye's middle row (row index 2).
        let mid = line_text(&scene(80, 5_100)[2]);
        assert!(mid.contains('━'), "no closed lid on a blink frame: {mid}");
        // ...and the level stare between events is NOT a blink.
        assert_eq!(idle_pose(2_000).0, Pose::Center);
    }

    #[test]
    fn direction_alternates_between_walks() {
        // First walk heads right, the next heads left.
        let prog = (20_000 - WALK_START) as f64 / (WALK_END - WALK_START) as f64;
        assert_eq!(walk_at(20_000, 0), Some((prog, true)));
        assert_eq!(walk_at(20_000, 1).map(|(_, r)| r), Some(false));
        assert_eq!(walk_at(6_000, 0), None);
    }

    #[test]
    fn runic_transliterates_and_keeps_the_thorn_digraph() {
        assert_eq!(runic("a b"), "ᚨ᛬ᛒ");
        // "th" collapses to a single thorn rune rather than two glyphs.
        assert_eq!(runic("th"), "ᚦ");
        assert_eq!(runic("teithant").chars().filter(|&c| c == 'ᚦ').count(), 1);
    }
}
