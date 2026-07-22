//! `sauron workspace` -- open a fullscreen iTerm2 multi-agent layout on its own
//! macOS Space, or manage the saved-project registry it launches from. This is
//! the whole of what used to be `workspace/workspace.sh`, moved into the binary
//! so there is one command and no shell-out: the in-flight sessions come
//! straight from the scanner, not from `sauron --list-working`.
//!
//! Left column  : one pane per in-flight session (Working or Delegated), each
//!                resumed with `claude --resume <id>`; extra panes are bare
//!                `claude`. Right column: sauron (top) + two shells at the repo.
//!
//! grep targets:
//!   fn run              -- entry point: alias subcommands, then launch
//!   fn resolve          -- a project arg (path or saved alias) -> repo dir
//!   fn store_* / alias  -- the name->path registry (~/.claude/sauron/workspaces)
//!   fn applescript      -- the iTerm layout script
//!   fn osascript        -- pipe the script to `osascript`

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::scan::home;

/// Entry point for `sauron workspace <args>` (args are everything after the
/// `workspace` word).
pub fn run(args: &[String]) -> std::io::Result<()> {
    // Registry subcommands run and return before any launch work.
    match args.first().map(|s| s.as_str()) {
        Some("alias") | Some("aliases") => {
            match (args.get(1), args.get(2)) {
                (Some(name), Some(path)) => alias_set(name, path)?,
                (Some(_), None) => {
                    eprintln!("usage: sauron workspace alias <name> <path>");
                    std::process::exit(2);
                }
                _ => alias_list(false),
            }
            return Ok(());
        }
        Some("unalias") | Some("forget") => {
            match args.get(1) {
                Some(name) => alias_del(name)?,
                None => {
                    eprintln!("usage: sauron workspace unalias <name>");
                    std::process::exit(2);
                }
            }
            return Ok(());
        }
        Some("ls") | Some("list") => {
            alias_list(false);
            return Ok(());
        }
        _ => {}
    }

    // Launch args, order-independent:  [init] [N] [project]. A purely-numeric
    // arg is the pane count; anything else is the project.
    let rest: &[String] = if args.first().map(|s| s.as_str()) == Some("init") {
        &args[1..]
    } else {
        args
    };
    let mut n_arg: Option<usize> = None;
    let mut project: Option<&str> = None;
    for a in rest {
        match a.parse::<usize>() {
            Ok(n) => n_arg = Some(n),
            Err(_) => project = Some(a),
        }
    }
    if n_arg == Some(0) {
        eprintln!("sauron workspace: agent count must be a positive integer");
        std::process::exit(1);
    }

    // Resolve which project to open. Explicit arg (path or alias) wins, else the
    // `default` alias, else $WORKSPACE_REPO, else the git repo of the cwd.
    let repo = match project {
        Some(p) => match resolve(p) {
            Some(r) => r,
            None => {
                eprintln!("sauron workspace: '{p}' is not a directory or a saved alias.");
                eprintln!("  saved aliases:");
                alias_list(true);
                std::process::exit(1);
            }
        },
        None => default_repo(),
    };
    if !repo.is_dir() {
        eprintln!("sauron workspace: repo not found: {}", repo.display());
        eprintln!("  pass a path or alias:  sauron workspace [N] <project>");
        eprintln!("  or save a default:     sauron workspace alias default /path/to/repo");
        std::process::exit(1);
    }

    let work = crate::in_flight_tasks(repo.clone());
    // Pane count: explicit arg wins; else one per in-flight task; else 4 bare.
    let total = n_arg.unwrap_or(if work.is_empty() { 4 } else { work.len() });

    // The panes run this very sauron binary for the TUI, by its real path, so a
    // restored iTerm session keeps resolving it.
    let sauron_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sauron"));
    let repo_s = repo.to_string_lossy().into_owned();

    // Dry run: report the plan and stop, before touching iTerm (used by tests
    // and handy for "what would `sauron workspace X` actually open?").
    if std::env::var_os("WORKSPACE_DRYRUN").is_some() {
        println!("REPO={repo_s}");
        println!("TOTAL={total}");
        println!("SAURON={}", sauron_exe.display());
        return Ok(());
    }

    let script = applescript(&repo_s, &sauron_exe.to_string_lossy(), total, &work);
    osascript(&script)?;

    let resumed = total.min(work.len());
    println!(
        "sauron workspace: opened {total}-pane layout on a new Space ({resumed} resumed working task(s), {} new) — repo: {repo_s}",
        total - resumed
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Workspace memory: a  name<TAB>path  registry so projects launch by a short
// alias. The special alias `default` is what a bare `sauron workspace` opens.
// ---------------------------------------------------------------------------

fn store_path() -> PathBuf {
    std::env::var_os("WORKSPACE_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".claude").join("sauron").join("workspaces"))
}

fn store_rows() -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(store_path()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let (n, p) = l.split_once('\t')?;
            (!n.is_empty()).then(|| (n.to_string(), p.to_string()))
        })
        .collect()
}

fn alias_lookup(name: &str) -> Option<String> {
    store_rows().into_iter().find(|(n, _)| n == name).map(|(_, p)| p)
}

fn write_rows(rows: &[(String, String)]) -> std::io::Result<()> {
    let path = store_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body: String = rows.iter().map(|(n, p)| format!("{n}\t{p}\n")).collect();
    std::fs::write(path, body)
}

fn alias_set(name: &str, raw: &str) -> std::io::Result<()> {
    let expanded = expand(raw);
    let abs = std::fs::canonicalize(&expanded).unwrap_or(expanded);
    if !abs.is_dir() {
        eprintln!("sauron workspace: not a directory: {}", abs.display());
        std::process::exit(1);
    }
    let abs = abs.to_string_lossy().into_owned();
    // Upsert: drop any existing row for this name, then append.
    let mut rows: Vec<_> = store_rows().into_iter().filter(|(n, _)| n != name).collect();
    rows.push((name.to_string(), abs.clone()));
    write_rows(&rows)?;
    println!("sauron workspace: alias '{name}' -> {abs}");
    Ok(())
}

fn alias_del(name: &str) -> std::io::Result<()> {
    let rows: Vec<_> = store_rows().into_iter().filter(|(n, _)| n != name).collect();
    write_rows(&rows)?;
    println!("sauron workspace: removed alias '{name}'");
    Ok(())
}

fn alias_list(to_stderr: bool) {
    let rows = store_rows();
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str("  (no workspaces saved yet — add one with: sauron workspace alias <name> <path>)\n");
    } else {
        for (n, p) in rows {
            out.push_str(&format!("  {n:<16} {p}\n"));
        }
    }
    if to_stderr {
        eprint!("{out}");
    } else {
        print!("{out}");
    }
}

/// Expand a leading `~` to the home directory.
fn expand(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix('~') {
        home().join(rest.trim_start_matches('/'))
    } else {
        PathBuf::from(raw)
    }
}

/// A project arg -> an absolute repo directory, or None.
fn resolve(project: &str) -> Option<PathBuf> {
    // Path-like (a slash, or . / .., or ~): strictly a directory.
    if project.contains('/') || project == "." || project == ".." || project.starts_with('~') {
        let p = expand(project);
        return std::fs::canonicalize(&p).ok().filter(|p| p.is_dir());
    }
    // A bare word means the alias first -- so `sauron workspace sauron` opens the
    // saved project, not a coincidental ./sauron subdir -- then a same-named dir.
    if let Some(hit) = alias_lookup(project) {
        let pb = PathBuf::from(hit);
        if pb.is_dir() {
            return Some(pb);
        }
    }
    let pb = PathBuf::from(project);
    std::fs::canonicalize(&pb).ok().filter(|p| p.is_dir())
}

fn default_repo() -> PathBuf {
    if let Some(r) = std::env::var_os("WORKSPACE_REPO") {
        return PathBuf::from(r);
    }
    if let Some(p) = alias_lookup("default") {
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            return pb;
        }
    }
    crate::git_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

// ---------------------------------------------------------------------------
// iTerm layout
// ---------------------------------------------------------------------------

/// Build the AppleScript that opens the window, fullscreens it onto its own
/// Space, and lays out the panes. Repo/exe paths are assumed quote-free (as the
/// shell version assumed), so they drop straight into the double-quoted strings.
fn applescript(repo: &str, sauron_exe: &str, total: usize, work: &[(String, String)]) -> String {
    let mut cmds = Vec::with_capacity(total);
    for i in 0..total {
        if i < work.len() {
            cmds.push(format!("\"cd {repo} && claude --resume {}\"", work[i].0));
        } else {
            cmds.push(format!("\"cd {repo} && claude\""));
        }
    }
    let cmds_list = cmds.join(", ");

    format!(
        r#"tell application "iTerm2"
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
  set cmds to {{{cmds_list}}}

  -- Carve the right column off the left, then stack it into 3 panes.
  tell leftTop to set rTop to (split vertically with default profile)
  tell rTop    to set rMid to (split horizontally with default profile)
  tell rMid    to set rBot to (split horizontally with default profile)

  tell rTop to write text "cd {repo} && {sauron_exe}"
  tell rMid to write text "cd {repo}"
  tell rBot to write text "cd {repo}"

  -- Left column: one pane per command. Split the CURRENTLY-TALLEST left pane
  -- each iteration (not the newest), so panes stay balanced instead of shrinking
  -- geometrically -- repeatedly splitting the newest pane drives it below iTerm2's
  -- minimum height, which throws and aborts the remaining splits.
  tell leftTop to write text (item 1 of cmds)
  set leftPanes to {{leftTop}}
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
"#
    )
}

/// Pipe the AppleScript to `osascript` on stdin, exactly as the shell heredoc did.
fn osascript(script: &str) -> std::io::Result<()> {
    let mut child = match Command::new("osascript").stdin(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sauron workspace: could not run osascript ({e}). macOS + iTerm2 only.");
            std::process::exit(1);
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(script.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        eprintln!("sauron workspace: osascript exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_lists_one_command_per_pane() {
        let work = vec![("id-1".to_string(), "task one".to_string())];
        let s = applescript("/repo", "/bin/sauron", 3, &work);
        // First pane resumes the working task; the rest are bare claude.
        assert!(s.contains("cd /repo && claude --resume id-1"));
        assert_eq!(s.matches("cd /repo && claude").count(), 3); // resume line counts too
        // The sauron pane runs this binary against the repo.
        assert!(s.contains("cd /repo && /bin/sauron"));
        // The command list is a well-formed AppleScript list.
        assert!(s.contains("set cmds to {\"cd /repo && claude --resume id-1\", "));
    }

    #[test]
    fn expand_handles_tilde() {
        assert_eq!(expand("/abs/path"), PathBuf::from("/abs/path"));
        assert!(expand("~/x").starts_with(home()));
        assert_eq!(expand("~"), home());
    }
}
