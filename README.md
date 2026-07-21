# agentwatch

A pair of tools for running several [Claude Code](https://claude.com/claude-code)
agents at once and keeping track of what they changed.

- **`agentwatch`** — a terminal sidecar that answers one question: *what did my
  agents change that I have not tested yet?* It reads the Claude Code session
  logs, flags each session as needing a test / blocked / clear, and lets you
  acknowledge work as you verify it. It writes nothing to your repo and never
  talks to a running agent.
- **`workspace`** — a macOS/iTerm2 launcher that opens a fullscreen multi-agent
  layout on its own Space: a column of bare `claude` panes plus `agentwatch` and
  a couple of shells.

The two are independent — `agentwatch` works on its own from any terminal;
`workspace` just wires it into a ready-made layout.

## agentwatch

### Build

```bash
cargo build --release --manifest-path agentwatch/Cargo.toml
```

The binary lands at `agentwatch/target/release/agentwatch`. Copy it onto your
`PATH` if you like:

```bash
cp agentwatch/target/release/agentwatch /usr/local/bin/
```

### Run

```bash
agentwatch                  # watch the repo containing the cwd
agentwatch /path/to/repo    # watch a specific repo
```

It reads `~/.claude/projects/<encoded-repo-path>/*.jsonl`, re-tailing every two
seconds (only appended bytes are parsed, so it stays cheap on large logs).

### Keys

| Key       | Action                                                        |
|-----------|---------------------------------------------------------------|
| `j` / `k` | move selection                                                |
| `a`       | acknowledge the selected session (mark its edits as tested)   |
| `u`       | un-acknowledge                                                |
| `b`       | baseline — ack the entire historical backlog, start empty     |
| `c`       | toggle showing cleared/idle sessions                          |
| `A`       | toggle showing the stale backlog                              |
| `q`       | quit                                                          |

Status ranking surfaces blocked and needs-test sessions first; cleared and idle
ones are counted but collapsed so today's outstanding work stays visible.

### How it reads sessions

Claude Code encodes a project path by replacing separators with dashes, e.g.
`/Users/you/code/my-repo` → `~/.claude/projects/-Users-you-code-my-repo/`.
agentwatch folds each session's `file-history-delta` records into a per-session
edit set, subtracts what you have acknowledged, and shows the remainder.

Acknowledgements persist in a small state file so restarting the tool does not
lose your progress.

## workspace (macOS + iTerm2)

```bash
workspace/workspace.sh          # 4 agent panes for the current repo
workspace/workspace.sh 8        # 8 agent panes
WORKSPACE_REPO=/path workspace/workspace.sh   # a specific repo
```

It creates a new iTerm2 window, puts it into native fullscreen (which gives it
its own macOS Space), then splits it into a left column of `claude` panes and a
right column with `agentwatch` on top.

By default the launcher looks for the binary at
`agentwatch/target/release/agentwatch` next to this repo; set `AGENTWATCH=/path`
or put `agentwatch` on your `PATH` to override.

### Requirements

- iTerm2 with the AppleScript API enabled
- Accessibility permission granted to iTerm2
  (System Settings → Privacy & Security → Accessibility) — needed for the
  fullscreen toggle.

## Layout

```
agentwatch/        Rust crate — the TUI sidecar
workspace/         iTerm2 multi-agent launcher
```
