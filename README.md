<div align="center">

# 🛰️ sauron

### Know what your agents changed — before you ship it.

A read-only sidecar for running **many coding agents at once** — [Claude Code](https://claude.com/claude-code) and [OpenAI Codex](https://github.com/openai/codex) — and never losing track of which ones left work you haven't tested.

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
- Quotes your **last message** to each session, and previews the **recent
  edits** of the selected one — re-brief at a glance
- And a lidless **Eye** keeps watch up top, in runes, while hobbits scurry past

</td>
<td width="50%" valign="top">

### 🪟 `sauron workspace`
A subcommand of the same binary. One command, a whole cockpit:

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

## 🪟 `sauron workspace` — the multi-agent cockpit

> **macOS + iTerm2 only.**

```bash
sauron workspace                  # the default project (see `alias default`)
sauron workspace 8                # 8 agent panes
sauron workspace 8 <project>      # a specific project — count & project any order
sauron workspace 8 .              # the current folder
sauron workspace 5 --orcs 2       # 5 hobbits + 2 orcs (see below)
```

### 👹 orcs — the maintenance swarm

`--orcs N` looses **N single-shot maintenance agents** into the *cold* corners of
the repo — the largest source files **no active session is touching** and nothing
has dirtied in git. Each orc gets one file and makes a focused pass: decompose it
if it's oversized, tighten structure, and clear its warnings, tests staying
green. They ride the right column beneath `sauron`, which watches them like any
other session. Where the **hobbits** do your directed quests, the orcs toil on
the plumbing you'd never get to — and they only ever take what's safe, so they
can't collide with a hobbit mid-edit.

`<project>` is a directory (path, `~`, `.`) **or** a short alias you've saved
into workspace memory:

```bash
sauron workspace alias default ~/code/my-repo   # what a bare `sauron workspace` opens
sauron workspace alias api ~/code/api-service   # then:  sauron workspace 6 api
sauron workspace alias                          # list saved aliases
sauron workspace unalias api                    # forget one
```

It spins up a new iTerm2 window, throws it into native fullscreen (which gives
it its own macOS Space), then splits it into a left column of `claude` panes and
a right column with `sauron` on top. The panes reopen each in-flight session
(`claude --resume`) pulled straight from the scanner — the same set the TUI
shows — and run `sauron` itself for the watcher, by the very path you invoked, so
a restored window keeps working. Registry lives at `~/.claude/sauron/workspaces`.

<details>
<summary><b>Requirements</b></summary>

<br>

- iTerm2 with the AppleScript API enabled
- Accessibility permission granted to iTerm2
  (**System Settings → Privacy & Security → Accessibility**) — needed for the
  fullscreen toggle.

</details>

---

## 🤖 Agents — Claude Code, Codex, and beyond

sauron isn't tied to one agent. Everything downstream — the status model, the
cards, workspace, orcs — is agent-agnostic; only *where the logs live* and *how
one record folds into a session* differ, behind a small `Agent` seam
(`src/agent.rs`). **Claude Code** is the default and fully supported; **OpenAI
Codex** is supported too; adding a third is a localized change.

### Choosing an agent

Pick with a flag, the `$SAURON_AGENT` env var, or let it auto-detect — first match wins:

| Precedence | Source | Example |
|:--|:--|:--|
| 1 | flag | `sauron --codex`, `sauron --claude` |
| 2 | env | `SAURON_AGENT=codex sauron` |
| 3 | auto-detect | whichever agent has logs for this repo |
| 4 | default | Claude Code |

The choice flows everywhere:

```bash
sauron                        # watch — auto-detect the agent
sauron --codex                # watch Codex sessions
sauron workspace 5 --codex    # a Codex cockpit: hobbit panes run `codex`,
                              #   the watcher runs `sauron --codex`
sauron workspace 5 --codex --orcs 2   # …and orcs run `codex exec`
```

### Codex specifics

sauron reads Codex rollouts from `~/.codex/sessions/**/*.jsonl`, matching them to
the repo by the `cwd` recorded in each rollout, and folds messages into
prompt/turn state and `apply_patch` envelopes into the write-set.

> ⚠️ **Codex support is best-effort and unverified.** It was written against the
> documented rollout format on a machine with no Codex install. It's defensive —
> it degrades rather than crashes — but if edits or prompts look off, **one real
> `~/.codex/sessions/**/rollout-*.jsonl`** pins the exact field names. The fix is
> isolated to `src/codex.rs`; reports very welcome.

### Adding another agent

Aider, Gemini CLI, Cursor, your own — see **[docs/AGENTS.md](docs/AGENTS.md)** for
the seam and a step-by-step. In short: add an `Agent` variant, give it a spawn
command and a log reader (session discovery + a `fold` that maps records onto the
shared `Session`), and the entire UI, workspace, and orc machinery come for free.

---

## 🔍 How it reads sessions

Each agent stores sessions its own way. **Claude Code** encodes a project path by
swapping separators for dashes:

```
/Users/you/code/my-repo   →   ~/.claude/projects/-Users-you-code-my-repo/
```

**Codex** writes dated rollouts under `~/.codex/sessions/`, tagged with the `cwd`
they ran in. Either way, sauron folds each session's records into a per-session
edit set, subtracts what you've acknowledged, and shows the remainder.
Acknowledgements live in a small state file, so restarting never loses your
place. It re-tails every two seconds, **writes nothing to your repo**, and
**never talks to a running agent**.

---

## 🗂️ Layout

```
sauron/src/
  agent.rs      ·  the agent seam — selection + spawn/log hooks
  scan.rs       ·  incremental log tailer + the Claude Code reader
  codex.rs      ·  the Codex rollout reader
  model.rs      ·  session model, status classification (agent-agnostic)
  ui.rs         ·  the TUI
  scene.rs      ·  the animated Eye
  workspace.rs  ·  the `sauron workspace` launcher (hobbits + orcs)
docs/AGENTS.md  ·  using Codex, and adding another agent
```

<div align="center">
<br>
<sub>Reads your logs. Touches nothing.</sub>
</div>
