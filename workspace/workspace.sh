#!/usr/bin/env bash
# workspace — open a fullscreen iTerm2 multi-agent layout on its own macOS Space.
#
# Left column  : one pane per currently-working sauron task, each resumed
#                with `claude --resume <session-id>`, split evenly (each pane =
#                1/N height, so window scale is proportional to the agent count).
#                Any panes beyond the working-task count are bare `claude`.
# Right column : sauron (top) + two plain shells at the repo root.
#
# "Currently working" is whatever sauron counts as Working — it is queried
# via `sauron --list-working`, so the set can never drift from the TUI.
#
# The window is put into native fullscreen via the Accessibility AXFullScreen
# attribute *before* splitting — macOS assigns a native-fullscreen window its own
# Space, the only reliable way to "open a new desktop" without a public Spaces
# API. Fullscreen-first also means large N never trips iTerm2's min pane size.
#
# Usage:
#   workspace                 open a pane for every working task (min 1)
#   workspace init            same
#   workspace 8               force 8 left panes: working tasks first, rest bare
#   workspace init 8          same
#
# When there are no working tasks and no count is given, opens 4 bare panes.
#
# Requirements:
#   - iTerm2 with the AppleScript API enabled
#   - Accessibility permission granted to iTerm2 (System Settings > Privacy &
#     Security > Accessibility) — needed for the AXFullScreen toggle.

set -euo pipefail

# The project to open the agent layout for. Defaults to the git repo containing
# the cwd, falling back to the cwd itself. Override with WORKSPACE_REPO.
REPO="${WORKSPACE_REPO:-$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null || pwd)}"

# Path to the sauron binary. Prefer the copy built alongside this script (the
# freshest build), then fall back to whatever `sauron` is on PATH -- e.g. from
# `cargo install --path sauron`, which drops it in ~/.cargo/bin -- so the
# launcher keeps working from a moved, clean, or relocated checkout instead of
# depending on a build living at one exact relative path. Override with SAURON.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCAL_SAURON="$SCRIPT_DIR/../sauron/target/release/sauron"
if [[ -n "${SAURON:-}" ]]; then
  : # explicit override wins, untouched
elif command -v sauron >/dev/null 2>&1; then
  # An installed copy (`cargo install --path sauron` -> ~/.cargo/bin) has a
  # stable absolute path, so the command this launcher bakes into each iTerm
  # pane keeps resolving after the repo is moved or a window is restored --
  # which a path into the build tree does not. This is the durable default.
  SAURON="$(command -v sauron)"
elif [[ -x "$LOCAL_SAURON" ]]; then
  SAURON="$LOCAL_SAURON" # no install yet: fall back to a fresh local build
else
  SAURON="$LOCAL_SAURON" # nothing built -> the "not built" hint points here
fi

# Accept:  workspace | workspace init | workspace 8 | workspace init 8
[[ "${1:-}" == "init" ]] && shift || true
N_ARG="${1:-}"

if [[ -n "$N_ARG" ]] && { ! [[ "$N_ARG" =~ ^[0-9]+$ ]] || (( N_ARG < 1 )); }; then
  echo "workspace: agent count must be a positive integer (got '$N_ARG')" >&2
  exit 1
fi

if [[ ! -d "$REPO" ]]; then
  echo "workspace: repo not found: $REPO" >&2
  echo "  set WORKSPACE_REPO=/path/to/repo to override" >&2
  exit 1
fi

# Pull the working tasks (id<TAB>name per line). Missing/unbuilt binary -> none.
WORK=()
if [[ -x "$SAURON" ]]; then
  while IFS= read -r line; do
    [[ -n "$line" ]] && WORK+=("$line")
  done < <("$SAURON" "$REPO" --list-working 2>/dev/null || true)
fi
W=${#WORK[@]}

# Pane count: explicit arg wins; else one per working task; else 4 bare.
if [[ -n "$N_ARG" ]]; then
  TOTAL="$N_ARG"
elif (( W > 0 )); then
  TOTAL="$W"
else
  TOTAL=4
fi

# Right-column top pane runs sauron if built, else a note.
if [[ -x "$SAURON" ]]; then
  SAURON_CMD="$SAURON"
else
  SAURON_CMD="echo 'sauron not built — run: cargo build --release --manifest-path $SCRIPT_DIR/../sauron/Cargo.toml'"
fi

# Build the AppleScript list of per-pane commands: working tasks (resumed) first,
# then bare claude for any remaining panes. IDs are uuids and REPO has no quotes,
# so no escaping is needed inside the double-quoted AppleScript strings.
resumed=0
CMDS=""
for (( i = 0; i < TOTAL; i++ )); do
  if (( i < W )); then
    id="${WORK[i]%%$'\t'*}"
    c="cd ${REPO} && claude --resume ${id}"
    resumed=$((resumed + 1))
  else
    c="cd ${REPO} && claude"
  fi
  CMDS+="\"${c}\", "
done
CMDS="{${CMDS%, }}"

osascript <<EOF
tell application "iTerm2"
  activate
  set w to (create window with default profile)
end tell
delay 0.6

-- Native fullscreen -> own Space. Target the frontmost (just-created) window.
tell application "System Events" to tell process "iTerm2"
  set value of attribute "AXFullScreen" of window 1 to true
end tell
delay 1.5

tell application "iTerm2"
  set t to current tab of w
  set leftTop to current session of t
  set cmds to ${CMDS}

  -- Carve the right column off the left, then stack it into 3 panes.
  tell leftTop to set rTop to (split vertically with default profile)
  tell rTop    to set rMid to (split horizontally with default profile)
  tell rMid    to set rBot to (split horizontally with default profile)

  tell rTop to write text "cd ${REPO} && ${SAURON_CMD}"
  tell rMid to write text "cd ${REPO}"
  tell rBot to write text "cd ${REPO}"

  -- Left column: one pane per command. Split the CURRENTLY-TALLEST left pane
  -- each iteration (not the newest), so panes stay balanced instead of shrinking
  -- geometrically — repeatedly splitting the newest pane drives it below iTerm2's
  -- minimum height, which throws and aborts the remaining splits.
  tell leftTop to write text (item 1 of cmds)
  set leftPanes to {leftTop}
  repeat with i from 2 to (count of cmds)
    set tallest to item 1 of leftPanes
    repeat with p in leftPanes
      if (rows of p) > (rows of tallest) then set tallest to contents of p
    end repeat
    tell tallest to set newP to (split horizontally with default profile)
    tell newP to write text (item i of cmds)
    set end of leftPanes to newP
  end repeat

  -- Land focus on the first agent pane.
  select leftTop
end tell
EOF

echo "workspace: opened ${TOTAL}-pane layout on a new Space (${resumed} resumed working task(s), $((TOTAL - resumed)) new) — repo: ${REPO}"
