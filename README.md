<div align="center">

# 👁️ sauron

### Know what your agents changed — before you ship it.

A read-only sidecar for running **many coding agents at once** — [Claude Code](https://claude.com/claude-code) and [OpenAI Codex](https://github.com/openai/codex) — and never losing track of which ones left work you haven't tested.

</div>

```
                    ▄▟█████▙▄
                  █▟███████▙█        The lidless Eye, wreathed in fire,
                  █▐███████▌█        watches every agent in your swarm
                  █▜███████▛█        — and tells you which one left work
 ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▟███████████▙▁▁▁▁    you still need to test.
```

<div align="center">

![Rust](https://img.shields.io/badge/built_with-Rust-CE422B?style=for-the-badge&logo=rust&logoColor=white)
![ratatui](https://img.shields.io/badge/TUI-ratatui-7C3AED?style=for-the-badge)
![Agents](https://img.shields.io/badge/agents-Claude_·_Codex-FF7A18?style=for-the-badge)
![macOS](https://img.shields.io/badge/workspace-macOS_·_iTerm2-000000?style=for-the-badge&logo=apple&logoColor=white)
![Read only](https://img.shields.io/badge/watcher_repo_writes-none-2EA043?style=for-the-badge)

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

The watcher reads each agent's session logs, re-tailing every two seconds. It
**writes nothing to your repo** and **never talks to a running agent**.

---

## 🎭 The cast

You are Sauron. Your servants get the work done; the Eye keeps watch.

| | Who | What they do |
|:-:|:--|:--|
| 🧝 | **hobbits** | the agents you direct — resumed into the `workspace` panes to do the real quests |
| 👹 | **orcs** | single-shot maintenance agents you loose on the *cold* corners: decompose, document, de-warn |
| 👁️ | **the Eye** | sauron itself, watching every session and flagging what needs you |
| 💍 | **Nazgûl** | *reserved* — the Nine are for something worse, later |

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
# 1 · build the sidecar and Agent Clipboard
cargo build --release --manifest-path sauron/Cargo.toml

# 2 · watch the repo you're standing in
./sauron/target/release/sauron

# …or watch a specific repo
./sauron/target/release/sauron /path/to/repo
```

Drop it on your `PATH` to call it from anywhere:

```bash
cp sauron/target/release/sauron sauron/target/release/clip /usr/local/bin/
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

## 📋 Agent Clipboard

Sauron includes a Rust implementation of the
[Forge Agent Clipboard](https://github.com/alvinlu7/forge). It preserves the
SQLite schema, FTS5 index, JSON record shape, TTL/version/checksum behavior, and
validated-before-pin policy, so the Python and Rust tools can share one database.

Use either entry point:

```bash
clip put project.current.invariants --file invariants.md --namespace project
clip search "project invariants" --namespace project --json
clip get project.current.invariants

# The same CLI is available through the Sauron binary.
sauron clip copy project.current.invariants
```

The complete command surface is `put`, `update`, `get`, `copy`, `search`, `list`,
`recent`, `pin`, `delete`, `export`, `import`, `stats`, `doctor`, and `gc`.
Override storage with `--db PATH` / `--db-path PATH` or
`$AGENT_CLIPBOARD_DB`. Otherwise Sauron reuses an ancestor
`.agent-clipboard/clipboard.sqlite3` when present, then falls back to the normal
platform data directory.

---

## 🪟 `sauron workspace` — the multi-agent cockpit

> **macOS + iTerm2 only.**

```bash
sauron workspace                  # the repo you're standing in, with a quick prompt
sauron workspace 8                # …suggesting 8 panes
sauron workspace 8 <project>      # a specific project — count & project any order
sauron workspace 8 .              # the current folder, explicitly
sauron workspace 5 --orcs 2       # 5 hobbits + 2 orcs (see below)
sauron workspace 5 --clipboard-handoff  # strict read-at-start/write-at-end passes
sauron workspace 8 <project> -y   # skip the prompt (also skipped when scripted)
```

A bare **`sauron workspace`** opens the **repository you're in** (the git repo
containing your cwd) and asks a quick question first:

```
  sauron workspace  →  /Users/you/code/worldsmith   (claude)
  panes [5]: 
  orcs  [0]: 
  launch 5 pane(s), 0 orc(s)? [Y/n]
```

Press Enter to accept each, type a number to change it, `q` to bail. Pass `-y`
(or pipe/redirect stdin) to skip the dialogue entirely.

### Strict clipboard handoffs

`--clipboard-handoff` wraps every launched hobbit and orc in an opt-in continuity
gate:

1. Before the agent starts, Sauron opens the clipboard, loads the exact prior
   lane handoff plus the highest-signal entries for this repository, and injects
   that context into the first prompt.
2. Before the agent exits, its instructions require a structured JSON handoff:
   completed work, repo/test state, invariants and decisions, blockers, and the
   precise next action.
3. After exit, Sauron reopens the database and verifies that the lane key has a
   new version containing the pass nonce. A missing or unreadable handoff marks
   the pass incomplete with exit code `3`.

Keys and namespaces are deterministic from the canonical repository path and
workspace lane, keeping parallel panes separate while allowing every later pass
to retrieve the repository's recent handoffs. Without the flag, workspace launch
commands and behavior are unchanged.

### 👹 orcs — the maintenance swarm

`--orcs N` stages **N single-shot maintenance agents** in the right column, aimed
at the *cold* corners of the repo — the largest source files **no active session
is touching** and nothing has dirtied in git. Each orc gets one file with a
focused brief: decompose it if it's oversized (**splitting it into a
well-documented nested module/filetree** where that's the natural shape), tighten
what remains, and clear its warnings — tests staying green.

- **They don't start on their own.** The command is *typed into each orc pane but
  not run* — you review the target and press **Enter** to loose it.
- **They're marked distinct.** An orc session wears a green **`orc`** badge in the
  TUI, so you can tell sauron's own maintenance work apart from the **hobbits**
  doing your directed quests.
- **They only take what's safe**, so they can never collide with a hobbit
  mid-edit.

`<project>` is a directory (path, `~`, `.`) **or** a short alias you've saved
into workspace memory:

```bash
sauron workspace alias api ~/code/api-service   # then:  sauron workspace 6 api
sauron workspace alias worldsmith ~/code/worldsmith
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
  clip/         ·  SQLite-compatible Agent Clipboard store + CLI
  handoff.rs    ·  strict opt-in clipboard pass lifecycle
  workspace.rs  ·  the `sauron workspace` launcher (hobbits + orcs)
docs/AGENTS.md  ·  using Codex, and adding another agent
```

<div align="center">
<br>
<sub>Reads your logs. Touches nothing.</sub>
</div>
