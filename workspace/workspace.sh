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
#   workspace                     open the default project (see `alias default`)
#   workspace 8                   force 8 left panes (working tasks first, rest bare)
#   workspace <project>           open a project by path or by saved alias
#   workspace 8 <project>         both — the count and project may be in either order
#   workspace 8 .                 open the current folder
#
#   workspace alias <name> <path> save a project alias into workspace memory
#   workspace alias               list saved aliases
#   workspace unalias <name>      forget one
#
# <project> is a directory (~ and . are fine) or an alias saved above. A bare
# `workspace` opens the `default` alias, else $WORKSPACE_REPO, else the git repo
# containing the cwd. When there are no working tasks and no count is given, it
# opens 4 bare panes.
#
# Requirements:
#   - iTerm2 with the AppleScript API enabled
#   - Accessibility permission granted to iTerm2 (System Settings > Privacy &
#     Security > Accessibility) — needed for the AXFullScreen toggle.

set -euo pipefail

# ---------------------------------------------------------------------------
# Workspace memory: a tiny  name<TAB>path  registry so projects can be launched
# by a short alias instead of a full path, and the default lives somewhere the
# next machine can be told about. The special alias `default` is what a bare
# `workspace` opens. Override the file location with WORKSPACE_STORE.
# ---------------------------------------------------------------------------
WS_STORE="${WORKSPACE_STORE:-$HOME/.claude/sauron/workspaces}"

# name -> path, empty if unknown. awk, not `grep -P`: BSD grep has no -P.
ws_alias_path() {
  [[ -f "$WS_STORE" ]] && awk -F'\t' -v n="$1" '$1==n{print $2; exit}' "$WS_STORE"
}

ws_alias_set() {
  local name="$1" path="$2"
  [[ -n "$name" && -n "$path" ]] || { echo "usage: workspace alias <name> <path>" >&2; exit 2; }
  path="${path/#\~/$HOME}"
  path="$(cd "$path" 2>/dev/null && pwd || echo "$path")" # absolute-ise if it exists
  [[ -d "$path" ]] || { echo "workspace: not a directory: $path" >&2; exit 1; }
  mkdir -p "$(dirname "$WS_STORE")"
  local tmp="$WS_STORE.tmp.$$"
  # Drop any existing row for this name, then append the new one -> upsert.
  { [[ -f "$WS_STORE" ]] && awk -F'\t' -v n="$name" '$1!=n' "$WS_STORE"
    printf '%s\t%s\n' "$name" "$path"; } > "$tmp"
  mv "$tmp" "$WS_STORE"
  echo "workspace: alias '$name' -> $path"
}

ws_alias_del() {
  local name="$1"
  [[ -n "$name" ]] || { echo "usage: workspace unalias <name>" >&2; exit 2; }
  [[ -f "$WS_STORE" ]] || { echo "workspace: no aliases saved"; return 0; }
  local tmp="$WS_STORE.tmp.$$"
  awk -F'\t' -v n="$name" '$1!=n' "$WS_STORE" > "$tmp"
  mv "$tmp" "$WS_STORE"
  echo "workspace: removed alias '$name'"
}

ws_alias_list() {
  if [[ -s "$WS_STORE" ]]; then
    awk -F'\t' '{printf "  %-16s %s\n", $1, $2}' "$WS_STORE"
  else
    echo "  (no workspaces saved yet — add one with: workspace alias <name> <path>)"
  fi
}

# Registry subcommands run and exit before any launch work.
case "${1:-}" in
  alias|aliases)
    shift
    if [[ $# -eq 0 ]]; then ws_alias_list; else ws_alias_set "${1:-}" "${2:-}"; fi
    exit 0 ;;
  unalias|forget)
    shift; ws_alias_del "${1:-}"; exit 0 ;;
  ls|list)
    ws_alias_list; exit 0 ;;
esac

# Launch args are order-independent:  workspace [init] [N] [project]
# A purely-numeric arg is the pane count; anything else is the project.
[[ "${1:-}" == "init" ]] && shift || true
N_ARG=""; PROJECT=""
for a in "$@"; do
  if [[ "$a" =~ ^[0-9]+$ ]]; then
    N_ARG="$a"
  else
    PROJECT="$a"
  fi
done

if [[ -n "$N_ARG" ]] && (( N_ARG < 1 )); then
  echo "workspace: agent count must be a positive integer (got '$N_ARG')" >&2
  exit 1
fi

# Path to the sauron binary. Prefer an installed copy on PATH (`cargo install
# --path sauron` -> ~/.cargo/bin): its stable absolute path keeps the command
# this launcher bakes into each iTerm pane resolving after the repo is moved or
# a window is restored, which a path into the build tree does not. Fall back to
# a fresh local build, then to the not-built hint. Override with SAURON.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCAL_SAURON="$SCRIPT_DIR/../sauron/target/release/sauron"
if [[ -n "${SAURON:-}" ]]; then
  : # explicit override wins, untouched
elif command -v sauron >/dev/null 2>&1; then
  SAURON="$(command -v sauron)"
elif [[ -x "$LOCAL_SAURON" ]]; then
  SAURON="$LOCAL_SAURON" # no install yet: fall back to a fresh local build
else
  SAURON="$LOCAL_SAURON" # nothing built -> the "not built" hint points here
fi

# Resolve which project to watch. Explicit arg wins (a directory path, or a
# saved alias); otherwise the `default` alias, then $WORKSPACE_REPO, then the
# git repo containing the cwd, then the cwd itself.
ws_resolve() { # $1 -> absolute dir on stdout, or non-zero
  local p="$1" hit
  p="${p/#\~/$HOME}"
  # Path-like (a slash, or . / ..): resolve strictly as a directory.
  if [[ "$p" == */* || "$p" == "." || "$p" == ".." ]]; then
    if [[ -d "$p" ]]; then (cd "$p" && pwd); return 0; fi
    return 1
  fi
  # A bare word means the alias first -- so `workspace sauron` opens the saved
  # sauron project, not a coincidental ./sauron subdir -- then a same-named dir.
  hit="$(ws_alias_path "$1")"
  if [[ -n "$hit" && -d "$hit" ]]; then echo "$hit"; return 0; fi
  if [[ -d "$p" ]]; then (cd "$p" && pwd); return 0; fi
  return 1
}

if [[ -n "$PROJECT" ]]; then
  if ! REPO="$(ws_resolve "$PROJECT")"; then
    echo "workspace: '$PROJECT' is not a directory or a saved alias." >&2
    echo "  saved aliases:" >&2
    ws_alias_list >&2
    exit 1
  fi
else
  REPO="${WORKSPACE_REPO:-$(ws_alias_path default)}"
  [[ -n "$REPO" ]] || REPO="$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null || pwd)"
fi

if [[ ! -d "$REPO" ]]; then
  echo "workspace: repo not found: $REPO" >&2
  echo "  pass a path or alias:  workspace [N] <project>" >&2
  echo "  or save a default:     workspace alias default /path/to/repo" >&2
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

# Dry run: report the resolved plan and stop, before touching iTerm. Used by the
# tests and handy for "what would `workspace X` actually open?".
if [[ -n "${WORKSPACE_DRYRUN:-}" ]]; then
  echo "REPO=$REPO"
  echo "TOTAL=$TOTAL"
  echo "SAURON=$SAURON"
  exit 0
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
