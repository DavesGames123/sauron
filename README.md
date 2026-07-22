<div align="center">

# 🛰️ sauron

### Know what your agents changed — before you ship it.

A read-only sidecar for running **many [Claude Code](https://claude.com/claude-code) agents at once** and never losing track of which ones left work you haven't tested.

<br>

![Rust](https://img.shields.io/badge/built_with-Rust-CE422B?style=for-the-badge&logo=rust&logoColor=white)
![ratatui](https://img.shields.io/badge/TUI-ratatui-7C3AED?style=for-the-badge)
![macOS](https://img.shields.io/badge/workspace-macOS_·_iTerm2-000000?style=for-the-badge&logo=apple&logoColor=white)
![Read only](https://img.shields.io/badge/repo_writes-none-2EA043?style=for-the-badge)

</div>

<br>

```
┌─ sauron ─────────────────────────────────────────────┐
│  ● letters-redesign    NEEDS TEST   src/gui/letters.rs    │
│  ▲ flora-field-tick    BLOCKED      waiting on you        │
│  ● combat-armory       NEEDS TEST   3 files               │
│  ○ crossword-fetch     clear                              │
└──────────────────────────────────────────────────────────┘
     j/k move   ·   a ack   ·   b baseline   ·   q quit
```

---

## Why

When you fan out a swarm of coding agents, the bottleneck stops being *writing*
code and becomes *trusting* it. Which sessions actually touched the repo? Which
are quietly **blocked on a question only you can answer**? What have you already
reviewed? **sauron** answers exactly that — and nothing else.

It reads the Claude Code session logs under
`~/.claude/projects/<repo>/*.jsonl`, re-tailing every two seconds. It **writes
nothing to your repo** and **never talks to a running agent**.

<br>

<table>
<tr>
<td width="50%" valign="top">

### 🛰️ `sauron`
The terminal sidecar. One question, answered live:
> *what did my agents change that I haven't tested yet?*

- Flags each session **needs-test · blocked · clear**
- **Acknowledge** work as you verify it — progress persists
- Surfaces sessions **blocked on a question** first
- Collapses the historical backlog so *today's* work stays visible

</td>
<td width="50%" valign="top">

### 🪟 `workspace`
The macOS/iTerm2 launcher. One command, a whole cockpit:

- Opens a **fullscreen layout on its own Space**
- A column of **bare `claude` panes** (pick how many)
- `sauron` wired into the top-right, shells below
- Panes stay evenly sized no matter the agent count

</td>
</tr>
</table>

---

## 🚀 Quick start

```bash
# 1 · build the sidecar
cargo build --release --manifest-path sauron/Cargo.toml

# 2 · watch the repo you're standing in
./sauron/target/release/sauron

# …or watch a specific repo
./sauron/target/release/sauron /path/to/repo
```

Drop it on your `PATH` to call it from anywhere:

```bash
cp sauron/target/release/sauron /usr/local/bin/
```

---

## ⌨️ Keys

| Key | Action |
|:---:|:-------|
| <kbd>j</kbd> / <kbd>k</kbd> | move selection |
| <kbd>a</kbd> | acknowledge selected session — mark its edits tested |
| <kbd>u</kbd> | un-acknowledge |
| <kbd>b</kbd> | **baseline** — ack the whole historical backlog, start empty |
| <kbd>c</kbd> | toggle cleared / idle sessions |
| <kbd>A</kbd> | toggle the stale backlog |
| <kbd>q</kbd> | quit |

---

## 🪟 workspace — the multi-agent cockpit

> **macOS + iTerm2 only.**

```bash
workspace/workspace.sh          # 4 agent panes for the current repo
workspace/workspace.sh 8        # 8 agent panes
WORKSPACE_REPO=/path workspace/workspace.sh   # a specific repo
```

It spins up a new iTerm2 window, throws it into native fullscreen (which gives
it its own macOS Space), then splits it into a left column of `claude` panes and
a right column with `sauron` on top.

By default the launcher looks for the binary next to this repo
(`sauron/target/release/sauron`). Override with `SAURON=/path`, or
just put `sauron` on your `PATH`.

<details>
<summary><b>Requirements</b></summary>

<br>

- iTerm2 with the AppleScript API enabled
- Accessibility permission granted to iTerm2
  (**System Settings → Privacy & Security → Accessibility**) — needed for the
  fullscreen toggle.

</details>

---

## 🔍 How it reads sessions

Claude Code encodes a project path by swapping separators for dashes:

```
/Users/you/code/my-repo   →   ~/.claude/projects/-Users-you-code-my-repo/
```

sauron folds each session's `file-history-delta` records into a per-session
edit set, subtracts what you've acknowledged, and shows the remainder.
Acknowledgements live in a small state file, so restarting never loses your
place.

---

## 🗂️ Layout

```
sauron/   ·  Rust crate — the TUI sidecar
workspace/    ·  iTerm2 multi-agent launcher
```

<div align="center">
<br>
<sub>Reads your logs. Touches nothing.</sub>
</div>
