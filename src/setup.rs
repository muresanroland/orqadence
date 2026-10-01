//! The thin 'orqa init': tidies a checkout an older init set up, installs
//! the shipped skills and every job's default in .orqadence/skills, offers
//! bd init, the docs/agents setup and herdr's integrations, keeps TypeSafe
//! on or off and its key, the shipped Ticket labels and their skills,
//! Rebase and Address PR comments' switches and the Release's, asks On
//! call's Moshi token, and preflights the Target repo.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::iter;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line as Row, Span};
use serde_json::{json, Value};

use crate::on_call;
use crate::orchestrator::app::{self, APPS};
use crate::orchestrator::state::{self, local_dir};
use crate::shell::brand::{self, BORDER, FRAME, GREEN, MUTED, PURPLE, TEXT, YELLOW};
use crate::skills::manifest::{self, Installed, Manifest, FILES, JOBS, LINKS};
use crate::skills::{stage_skill, CREATE_PR, EXTRA_FILES, SKILLS};
use crate::tools::Tools;

/// The record of every skill file init wrote, path to the text it wrote:
/// under refresh, a file that still matches is unedited and is rewritten.
const RECORD: &str = ".orqadence/installed-skills.json";
pub(crate) const KEY_FILE: &str = ".orqadence-local/typesafe-key";

#[derive(Clone, Copy)]
enum Mode {
    Fresh,
    Refresh,
    Overwrite,
}

/// The run files an init from before .orqadence-local left in .orqadence.
const OLD_RUN_FILES: [&str; 5] = [
    "state.json",
    "lock",
    "orchestrator.log",
    "runs",
    "worktrees",
];

/// Tidies a checkout an init from before .orqadence-local set up (ADR 0006),
/// before the skills step. The run files it left in .orqadence are not
/// moved: a saved Stage's conversation holds their paths. So one question,
/// the worktrees with uncommitted changes named: yes removes the worktrees,
/// their branches kept (one git fails to remove is said, its folder
/// deleted), and deletes the rest; no, or nobody answering,
/// stops init with everything left as it was. Then the .orqadence/ line
/// that init added to the Target repo's .gitignore goes, since .orqadence
/// is committed now.
pub(crate) fn clean_old_checkout(
    repo: &Path,
    tools: &dyn Tools,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    let old = repo.join(".orqadence");
    // ponytail: a Shell locking in this same instant has no pid written yet,
    // reads as 0 and is not refused; nobody starts an old Shell mid-init.
    if state::lock_file_holder(&old.join("lock")) != 0 {
        return Err(io::Error::other(
            "a Shell is running in this checkout; close it, then run orqa init again",
        ));
    }
    let leftovers: Vec<&str> = OLD_RUN_FILES
        .into_iter()
        .filter(|name| fs::symlink_metadata(old.join(name)).is_ok())
        .collect();
    if !leftovers.is_empty() {
        let worktrees: Vec<PathBuf> = fs::read_dir(old.join("worktrees"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        step(out, named("OLD RUN FILES"))?;
        write!(
            out,
            "init: an older Orqadence left its run files in .orqadence:\r\n"
        )?;
        for name in &leftovers {
            write!(out, "  .orqadence/{name}\r\n")?;
        }
        for worktree in &worktrees {
            // A status that fails cannot say it is clean. The flag overrides a
            // status.showUntrackedFiles=no that would hide untracked files.
            let status = ["git", "status", "--porcelain", "--untracked-files=normal"];
            if tools
                .run(worktree, &status)
                .map_or(true, |status| !status.trim().is_empty())
            {
                let name = worktree.strip_prefix(repo).unwrap_or(worktree);
                write!(out, "  {} has uncommitted changes\r\n", name.display())?;
            }
        }
        let question =
            "Delete them? The worktrees are removed, their branches kept, and a saved run is lost.";
        if yes(out, input, tty, question, false)? != Some(true) {
            return Err(io::Error::other(
                "stopped: the old run files in .orqadence are kept; run orqa init again and answer y to delete them",
            ));
        }
        let mut failed = Vec::new();
        for worktree in &worktrees {
            let path = worktree.display().to_string();
            // Force twice removes a locked one too. One git no longer knows,
            // say: its folder goes with the rest.
            let remove = ["git", "worktree", "remove", "--force", "--force", &path];
            if let Err(err) = tools.run(repo, &remove) {
                write!(out, "init: {err}; deleting its folder anyway\r\n")?;
                failed.push(path);
            }
        }
        for name in leftovers {
            let path = old.join(name);
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        // A removal that failed may have left git's record of a now missing
        // folder, its branch still checked out there: removing it again drops
        // that record, and no other worktree's, as a bare prune would.
        for path in failed {
            let remove = ["git", "worktree", "remove", "--force", "--force", &path];
            if let Err(err) = tools.run(repo, &remove) {
                write!(out, "init: {err}\r\n")?;
            }
        }
    }
    if remove_lines(&repo.join(".gitignore"), &[".orqadence/".to_string()])? {
        write!(
            out,
            "init: removed .orqadence/ from .gitignore: its settings are committed now\r\n"
        )?;
    }
    Ok(())
}

/// Whether git tracks .orqadence/config.json: the repo's settings are
/// committed, so init asks only this machine's and this person's questions
/// (ADR 0006). The later team questions check the same flag. Tracked, git
/// prints the path.
pub(crate) fn committed(repo: &Path, tools: &dyn Tools) -> bool {
    tools
        .run(repo, &["git", "ls-files", ".orqadence/config.json"])
        .is_ok_and(|listed| !listed.trim().is_empty())
}

/// Writes Orqadence's skills to .orqadence/skills, linked from
/// .agents/skills and .claude/skills (ADR 0006), after putting there those
/// an older init left elsewhere (manifest::settle) and taking out the lines
/// that hid the links from git. When the shipped skills are already
/// installed the gate asks first: cancel (false: no skill's text touched),
/// refresh only the files unedited since install (by the record), or
/// overwrite everything; force is overwrite unasked. A Shipped skill the
/// repo has in its own .agents/skills moves into .orqadence/skills first,
/// where the gate decides it, and every skill an init installed before
/// manifest::PREFIX is renamed to its name with it, the record too.
/// `input` answers the questions, in raw mode when `tty`. .orqadence-local is made first, whatever the answers: orqa
/// opens once it exists; a TypeSafe key an older init kept in .orqadence
/// moves into it, readable only by the user, or is deleted when one is
/// already there, since that one wins.
pub(crate) fn install_skills(
    repo: &Path,
    home: &Path,
    force: bool,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<bool> {
    local_dir(repo)?;
    let old_key = repo.join(".orqadence/typesafe-key");
    if old_key.exists() && !repo.join(KEY_FILE).exists() {
        fs::rename(old_key, repo.join(KEY_FILE))?;
    } else if old_key.exists() {
        // The kept key wins; the old one, no longer ignored, could be committed.
        fs::remove_file(old_key)?;
    }
    // Also repairs a key an earlier init moved without narrowing its mode.
    if repo.join(KEY_FILE).exists() {
        fs::set_permissions(repo.join(KEY_FILE), fs::Permissions::from_mode(0o600))?;
    }
    let mut record: BTreeMap<String, String> = fs::read_to_string(repo.join(RECORD))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let mut manifest = Manifest::load(repo).map_err(io::Error::other)?;
    // What an older init recorded only in its record is Orqadence's too.
    for name in record.keys().filter_map(|key| {
        key.strip_prefix(".agents/skills/")?
            .strip_suffix("/SKILL.md")
    }) {
        manifest.skills.entry(name.to_string()).or_insert(shipped());
    }
    manifest::settle(repo, home, &manifest)?;
    unhide_links(repo)?;
    let (renamed, left) = manifest::prefix(repo, &mut manifest)?;
    for (old, new) in &renamed {
        if let Some(wrote) = record.remove(&record_key(old, "SKILL.md")) {
            record
                .entry(record_key(new, "SKILL.md"))
                .or_insert_with(|| manifest::renamed(&wrote, new));
        }
        write!(out, "init: renamed {old} to {new}\r\n")?;
    }
    for (old, new) in &left {
        write!(
            out,
            "init: {old} is not renamed: {new} is there too; remove one of them\r\n"
        )?;
    }
    // Saved now: a cancel at the gate must not leave the folders renamed
    // and the manifest naming the old ones.
    if !renamed.is_empty() {
        save_skills(repo, &record, &manifest)?;
    }
    let mode = if force {
        Mode::Overwrite
    } else if installed(&repo.join(FILES)) {
        step(out, named("SKILLS"))?;
        let question = "The shipped skills are already installed here.";
        let options = [
            "cancel, leave their text as it is",
            "refresh only the skills not edited since install",
            "overwrite everything with the shipped skills",
        ];
        match raw(tty, || {
            choose(out, &mut *input, question, &options, (0, 0), "")
        })? {
            None | Some(0) => return Ok(false),
            Some(1) => Mode::Refresh,
            _ => Mode::Overwrite,
        }
    } else {
        Mode::Fresh
    };
    for &(name, body) in SKILLS {
        manifest::move_in(repo, name)?;
        let dir = manifest::skill_dir(repo, name);
        let is_link = |path: &Path| {
            fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
        };
        // A link is the repo's own arrangement: never written through.
        if is_link(&dir.join("SKILL.md")) {
            continue;
        }
        let extra = EXTRA_FILES
            .iter()
            .filter(|(skill, ..)| *skill == name)
            .map(|&(_, file, body)| (file, body));
        for (file, body) in iter::once(("SKILL.md", body)).chain(extra) {
            let (rel, dest) = (record_key(name, file), dir.join(file));
            // The file or any folder on its way, as a linked scripts/.
            let linked = Path::new(file)
                .ancestors()
                .any(|part| !part.as_os_str().is_empty() && is_link(&dir.join(part)));
            if linked {
                continue;
            }
            let write = match mode {
                Mode::Overwrite => true,
                // A file not there yet, as a skill newer than the install, is
                // installed; one there is rewritten only when unedited.
                Mode::Refresh => {
                    !dest.exists()
                        || record.get(&rel).is_some_and(|wrote| {
                            fs::read_to_string(&dest).is_ok_and(|now| now == *wrote)
                        })
                }
                Mode::Fresh => !dest.exists(),
            };
            if write {
                fs::create_dir_all(dest.parent().unwrap())?;
                fs::write(&dest, body)?;
                if file != "SKILL.md" {
                    fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
                }
                record.insert(rel, body.to_string());
                manifest.skills.insert(name.to_string(), shipped());
            }
        }
        manifest::link(repo, name)?;
    }
    save_skills(repo, &record, &manifest)?;
    Ok(true)
}

/// Writes init's record and the Skill manifest.
fn save_skills(
    repo: &Path,
    record: &BTreeMap<String, String>,
    manifest: &Manifest,
) -> io::Result<()> {
    fs::create_dir_all(repo.join(".orqadence"))?;
    fs::write(
        repo.join(RECORD),
        serde_json::to_string_pretty(record)? + "\n",
    )?;
    manifest.save(repo).map_err(io::Error::other)
}

/// A Shipped skill's manifest entry.
fn shipped() -> Installed {
    Installed {
        shipped: true,
        ..Installed::default()
    }
}

/// The record's key for a skill's file init wrote: its path in the repo's
/// .agents/skills, as the record has always named it, wherever it is now.
fn record_key(name: &str, file: &str) -> String {
    format!(".agents/skills/{name}/{file}")
}

/// Whether a shipped skill is already installed in dir.
fn installed(dir: &Path) -> bool {
    SKILLS
        .iter()
        .any(|&(name, _)| dir.join(name).join("SKILL.md").exists())
}

/// Takes out the lines an older orqa put in .git/info/exclude to hide each
/// skill's links in a Ticket's worktree: the committed links in the
/// checkout would be hidden too. Only those, for the skills in
/// .orqadence/skills, as it wrote them; the rest stay.
fn unhide_links(repo: &Path) -> io::Result<()> {
    let hidden: Vec<String> = fs::read_dir(repo.join(FILES))
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            LINKS.map(|sub| format!("/{sub}/{name}"))
        })
        .collect();
    remove_lines(&repo.join(".git/info/exclude"), &hidden)?;
    Ok(())
}

/// The rest of init once the skills are in: bd, the docs/agents setup,
/// TypeSafe, every job's default, the Ticket labels, Rebase and Address PR
/// comments' switches, the Release's, On call, and herdr's integrations. A
/// `committed` checkout keeps the committed TypeSafe switch, picks, labels
/// and switches, and is asked only for the key, when TypeSafe is on and no
/// key is set or kept; On call is per person, asked on every checkout.
pub(crate) fn set_up(
    repo: &Path,
    tools: &dyn Tools,
    env: &dyn Fn(&str) -> String,
    committed: bool,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    let env_key = env("TYPESAFE_API_KEY");
    bd_init(repo, tools, out, input, tty)?;
    write_agent_docs(repo, out, input, tty)?;
    if !committed {
        let typesafe = ask_typesafe(repo, &env_key, out, input, tty)?;
        install_defaults(repo, tools, typesafe, out)?;
        ask_labels(repo, tools, out, input, tty)?;
        ask_switches(repo, out, input, tty)?;
        ask_release(repo, out, input, tty)?;
    } else if app::typesafe(repo) && env_key.trim().is_empty() && !repo.join(KEY_FILE).exists() {
        ask_typesafe_key(repo, out, input, tty)?;
    }
    ask_on_call(repo, &env(on_call::TOKEN_VAR), out, input, tty)?;
    install_integrations(repo, tools, out, input, tty)
}

/// Rebase and Address PR comments by themselves, each a yes/no kept in
/// config.json, its default the switch as it is: off on a first init, so
/// enter alone is no, and a re-run keeps a switch already set. Nobody
/// answering keeps it too. Then Agent merge (ask_agent_merge).
fn ask_switches(
    repo: &Path,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    step(out, named("PULL REQUESTS"))?;
    for switch in [&app::REBASE_AUTO, &app::ADDRESS_PR_COMMENTS_AUTO] {
        let on = app::switch(repo, switch);
        let on = yes(out, input, tty, switch.question, on)?.unwrap_or(on);
        app::set_switch(repo, switch, on).map_err(io::Error::other)?;
    }
    ask_agent_merge(repo, out, input, tty)
}

/// Agent merge, asked only with automatic Address PR comments on, its
/// default the switch as it is, as ask_switches' are; without it init says
/// why it is off, set_switch having turned off one kept on. Yes asks which
/// review bots the repo has, ticked as config.json lists them, kept when
/// the ticks change; bot_wait keeps its default.
fn ask_agent_merge(
    repo: &Path,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    let (_, doc) = app::read(repo).map_err(io::Error::other)?;
    if !app::switch_in(&doc, &app::ADDRESS_PR_COMMENTS_AUTO) {
        return note(out, app::AGENT_MERGE_NEEDS);
    }
    let on = app::switch_in(&doc, &app::AGENT_MERGE);
    let question = format!(
        "{} (security, db and infra Tickets still wait for you)",
        app::AGENT_MERGE.question
    );
    let on = yes(out, input, tty, &question, on)?.unwrap_or(on);
    app::set_switch(repo, &app::AGENT_MERGE, on).map_err(io::Error::other)?;
    if !on {
        return Ok(());
    }
    let kept = app::review_bots_in(&doc).unwrap_or_default();
    let rows = app::REVIEW_BOTS.map(|bot| (bot, String::new()));
    let ticked = app::REVIEW_BOTS.map(|bot| kept.contains(&bot));
    let question = "Which review bots does this repo have?";
    let ticked = raw(tty, || checklist(out, input, question, &rows, &ticked))?;
    let bots: Vec<&str> = app::REVIEW_BOTS
        .into_iter()
        .zip(ticked)
        .filter_map(|(bot, on)| on.then_some(bot))
        .collect();
    // Unchanged, a list that cannot be read stays for /config to flag.
    if bots == kept {
        return Ok(());
    }
    app::set_review_bots(repo, &bots).map_err(io::Error::other)
}

/// Releases (the orqa:release label), a yes/no kept in config.json, default
/// yes: enter alone turns it on, and nobody answering turns it on only while
/// unset; a re-run nobody answers keeps a switch already set.
fn ask_release(
    repo: &Path,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    step(out, named("RELEASES"))?;
    let (_, doc) = app::read(repo).map_err(io::Error::other)?;
    let on = doc[app::RELEASE_ON.key].is_null() || app::switch_in(&doc, &app::RELEASE_ON);
    let on = yes(out, input, tty, app::RELEASE_ON.question, true)?.unwrap_or(on);
    app::set_switch(repo, &app::RELEASE_ON, on).map_err(io::Error::other)
}

/// On call's opt-in, default no: yes asks for the Moshi token with echo off
/// and keeps it in the per-person config.json. No, Ctrl-C, Ctrl-D, an empty
/// token or nobody answering keeps none. A token kept, or MOSHI_WEBHOOK_TOKEN
/// (`env_token`) set, is not asked again.
fn ask_on_call(
    repo: &Path,
    env_token: &str,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    let mut kept = on_call::load(repo, &|_| String::new());
    if kept.token.is_some() || !env_token.trim().is_empty() {
        return write!(out, "init: On call on\r\n");
    }
    step(out, brand::chip("moshi"))?;
    note(out, &format!("see {}", on_call::DOC))?;
    let question = "Ring your phone through Moshi when a Question waits?";
    if yes(out, input, tty, question, false)? != Some(true) {
        return Ok(());
    }
    let what = "Moshi token, from the Moshi app's Settings > Notifications";
    let token = secret(out, input, tty, "token", what)?;
    if token.is_empty() {
        return Ok(());
    }
    kept.token = Some(token);
    if let Err(err) = on_call::save(repo, &kept) {
        return write!(out, "init: On call off, the token not kept: {err}\r\n");
    }
    write!(
        out,
        "init: On call on: Moshi token kept in {}, readable only by you\r\n",
        on_call::CONFIG
    )
}

/// Offers herdr's integration, which reports each session's id, for every
/// App on PATH whose integration herdr says is not installed or outdated:
/// what each install writes, then one question. Declined, or nobody
/// answering, those Stages cannot be resumed by id.
fn install_integrations(
    repo: &Path,
    tools: &dyn Tools,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    // herdr 0.9.1 prints it as text only: "codex: outdated (v7) (<path>)".
    let status = match tools.run(repo, &["herdr", "integration", "status"]) {
        Ok(status) => status,
        Err(err) => {
            write!(
                out,
                "init: herdr integration status failed, so no integration was offered and /continue may start Stages fresh; fix herdr and run orqa init again: {err}\r\n"
            )?;
            return Ok(());
        }
    };
    let stale: Vec<(&str, &str)> = APPS
        .iter()
        .filter_map(|app| {
            let state = status
                .lines()
                .find_map(|line| line.strip_prefix(app.name)?.strip_prefix(": "))?;
            if !state.starts_with("not installed") && !state.starts_with("outdated") {
                return None;
            }
            tools.run(repo, &["which", app.bin]).ok()?;
            Some((app.name, state.trim_end()))
        })
        .collect();
    if stale.is_empty() {
        return Ok(());
    }
    step(out, brand::chip("herdr"))?;
    write!(
        out,
        "init: herdr's integration tells herdr each session's id, so /continue can resume a Stage; it writes a hook script and registers it in the App's settings:\r\n"
    )?;
    for (name, state) in &stale {
        let row = vec![
            "    ".into(),
            brand::chip(name),
            Span::styled(format!("  {state}"), MUTED),
        ];
        write!(out, "{}\r\n", paint(row))?;
    }
    let question = "Install herdr's integration for these?";
    if yes(out, input, tty, question, true)? == Some(true) {
        for (name, _) in &stale {
            match tools.run(repo, &["herdr", "integration", "install", name]) {
                Ok(_) => write!(out, "init: installed herdr's {name} integration\r\n")?,
                Err(err) => write!(
                    out,
                    "init: herdr's {name} integration not installed: {err}\r\n"
                )?,
            }
        }
    } else {
        let names: Vec<&str> = stale.iter().map(|(name, _)| *name).collect();
        write!(
            out,
            "init: skipped: Stages on {} cannot be resumed by id, so /continue starts them fresh\r\n",
            names.join(", ")
        )?;
    }
    Ok(())
}

/// With no bd workspace, offers to run bd init. Nobody answering skips it,
/// and the preflight says it is missing.
fn bd_init(
    repo: &Path,
    tools: &dyn Tools,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    if repo.join(".beads").exists() {
        return Ok(());
    }
    step(out, brand::chip("beads"))?;
    if yes(
        out,
        input,
        tty,
        "No bd workspace here. Run bd init now?",
        true,
    )? != Some(true)
    {
        return Ok(());
    }
    match tools.run(repo, &["bd", "init", "--non-interactive"]) {
        Ok(_) => write!(out, "init: bd init done\r\n"),
        Err(err) => write!(out, "init: bd init failed: {err}\r\n"),
    }
}

/// The beads docs/agents setup init writes, compiled in: this repo's own.
const AGENT_DOCS: [(&str, &str); 3] = [
    (
        "docs/agents/issue-tracker.md",
        include_str!("../docs/agents/issue-tracker.md"),
    ),
    (
        "docs/agents/triage-labels.md",
        include_str!("../docs/agents/triage-labels.md"),
    ),
    (
        "docs/agents/domain.md",
        include_str!("../docs/agents/domain.md"),
    ),
];

/// The Agent skills block, modelled on this repo's AGENTS.md.
const AGENT_SKILLS: &str = "## Agent skills

### Issue tracker

Issues live in beads (`bd`), not GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default triage roles, each a bd label of the same name. See `docs/agents/triage-labels.md`.

### Domain docs

`CONTEXT.md` and `docs/adr/` at the root. See `docs/agents/domain.md`.
";

/// Writes what the repo lacks of the beads docs/agents setup, asked first;
/// a non-interactive init writes it too. The Agent skills block goes into
/// CLAUDE.md, else AGENTS.md, unless either has it already.
fn write_agent_docs(
    repo: &Path,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    let docs: Vec<_> = AGENT_DOCS
        .iter()
        .filter(|(path, _)| !repo.join(path).exists())
        .collect();
    let has_block = ["CLAUDE.md", "AGENTS.md"].iter().any(|file| {
        fs::read_to_string(repo.join(file)).is_ok_and(|text| text.contains("## Agent skills"))
    });
    if docs.is_empty() && has_block {
        return Ok(());
    }
    step(out, named("DOCS"))?;
    let question = "Write the beads docs/agents setup (issue tracker, triage labels, domain, Agent skills block)?";
    if yes(out, input, tty, question, true)? == Some(false) {
        return Ok(());
    }
    for (path, text) in docs {
        let at = repo.join(path);
        fs::create_dir_all(at.parent().unwrap())?;
        fs::write(&at, text)?;
        write!(out, "init: wrote {path}\r\n")?;
    }
    if !has_block {
        let file = if repo.join("CLAUDE.md").exists() {
            "CLAUDE.md"
        } else {
            "AGENTS.md"
        };
        let mut text = match fs::read_to_string(repo.join(file)) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err),
        };
        if !text.is_empty() {
            text += if text.ends_with('\n') { "\n" } else { "\n\n" };
        }
        fs::write(repo.join(file), text + AGENT_SKILLS)?;
        write!(out, "init: wrote the Agent skills block into {file}\r\n")?;
    }
    Ok(())
}

/// The typesafe-ai skill init installs when TypeSafe is on: (name, source).
pub(crate) const TYPESAFE_SKILL: (&str, &str) =
    ("orqa-typesafe-ai", "typesafe-ai/skills/skills/typesafe-ai");

/// Installs every job's default the manifest lacks, and the typesafe-ai
/// skill when TypeSafe is on, in .orqadence/skills, each pinned by its
/// commit. A failure is said and init goes on: the preflight names the job.
pub(crate) fn install_defaults(
    repo: &Path,
    tools: &dyn Tools,
    typesafe: bool,
    out: &mut dyn Write,
) -> io::Result<()> {
    let defaults = JOBS.iter().map(|(job, suggestions)| {
        (
            suggestions[0],
            format!("the {} default", job.replace('-', " ")),
        )
    });
    let typesafe = typesafe.then(|| (TYPESAFE_SKILL, "for TypeSafe".to_string()));
    install_missing(repo, tools, out, defaults.chain(typesafe))
}

/// Installs each ((name, source), what) the manifest lacks, in
/// .orqadence/skills, pinned by its commit. An empty source is built into
/// an App or Shipped: nothing to install. A failure is said and init goes
/// on: the preflight names what is missing.
fn install_missing<'a>(
    repo: &Path,
    tools: &dyn Tools,
    out: &mut dyn Write,
    wanted: impl Iterator<Item = ((&'a str, &'a str), String)>,
) -> io::Result<()> {
    for ((name, source), what) in wanted {
        // Loaded each time: add saves the manifest.
        let manifest = Manifest::load(repo).map_err(io::Error::other)?;
        if source.is_empty() || manifest.skills.contains_key(name) {
            continue;
        }
        match manifest::add(repo, tools, source, Some(name)) {
            Ok(_) => writeln!(out, "init: installed {name}, {what}")?,
            Err(err) => writeln!(out, "init: {name} not installed: {err}")?,
        }
    }
    Ok(())
}

/// A shipped Ticket label: the entry init writes to config.json's labels,
/// and where its skills come from.
pub(crate) struct ShippedLabel {
    /// The part after orqa:.
    pub(crate) name: &'static str,
    /// "area" or "modifier".
    pub(crate) kind: &'static str,
    /// A human merges its Tickets' PRs.
    pub(crate) human_merge: bool,
    /// (installed name, manifest source), installed as the jobs' defaults are.
    pub(crate) skills: &'static [(&'static str, &'static str)],
    /// The line the code-editing Stages get as Label guidance.
    pub(crate) guidance: &'static str,
    /// The Extra review, every Round: (skill, source, debated). An empty
    /// source is a Shipped skill.
    pub(crate) review: Option<(&'static str, &'static str, bool)>,
    /// Row overrides: (row, field, value).
    pub(crate) rows: &'static [(&'static str, &'static str, &'static str)],
    /// The section its PR template adds to the default: a heading and a
    /// comment saying what goes there. Empty for a Modifier label.
    pub(crate) pr_section: &'static str,
}

impl ShippedLabel {
    /// Its PR template's file name in TEMPLATE_DIR.
    fn template_file(&self) -> String {
        format!("{}.md", self.name)
    }

    /// Every skill the label needs from a source, its Extra review's too:
    /// (installed name, manifest source).
    pub(crate) fn sources(&self) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
        let review = self.review.map(|(name, source, _)| (name, source));
        self.skills.iter().copied().chain(review)
    }

    /// The entry as config.json's labels holds it: human_merge, rows and
    /// extra_review only where set.
    fn entry(&self) -> Value {
        let skills: Vec<&str> = self.skills.iter().map(|(name, _)| *name).collect();
        let mut entry = json!({"kind": self.kind, "skills": skills, "guidance": self.guidance});
        if self.human_merge {
            entry["human_merge"] = json!(true);
        }
        for (row, field, value) in self.rows {
            entry["rows"][row][field] = json!(value);
        }
        if let Some((skill, _, debate)) = self.review {
            entry["extra_review"] = json!({"skill": skill, "position": "every", "debate": debate});
        }
        entry
    }
}

/// The shipped Ticket labels (harness-bsg.7, .8 and .21), in the order
/// init lists them. A human merges the PRs of db, security and infra
/// Tickets (harness-72t.2).
pub(crate) const LABELS: [ShippedLabel; 7] = [
    ShippedLabel {
        name: "fe",
        kind: "area",
        human_merge: false,
        skills: &[("orqa-frontend-design", "anthropics/skills/skills/frontend-design")],
        guidance: "Front-end work: the pull request carries screenshots of every changed screen.",
        review: None,
        rows: &[],
        pr_section: "## Screenshots\n<!-- every changed screen after the change, attached; light and dark only if theming changed; a short video for a new flow; if the app cannot start, why -->\n",
    },
    ShippedLabel {
        name: "be",
        kind: "area",
        human_merge: false,
        skills: &[("orqa-api-and-interface-design", "addyosmani/agent-skills/skills/api-and-interface-design")],
        guidance: "Keep the contract explicit: requests, responses, status codes, compatibility.",
        review: None,
        rows: &[],
        pr_section: "## Contract\n<!-- before and after request and response, status codes, a compatibility note; an oasdiff changelog if there is an OpenAPI spec and oasdiff is installed, else by hand from the diff; a Mermaid sequenceDiagram if async -->\n",
    },
    ShippedLabel {
        name: "db",
        kind: "area",
        human_merge: true,
        skills: &[
            ("orqa-supabase-postgres-best-practices", "supabase/agent-skills/skills/supabase-postgres-best-practices"),
            ("orqa-deprecation-and-migration", "addyosmani/agent-skills/skills/deprecation-and-migration"),
        ],
        guidance: "Expand/contract migrations; say what locks or rewrites a table; give a rollback path.",
        review: None,
        rows: &[],
        pr_section: "## Schema\n<!-- a Mermaid erDiagram of the touched tables; locks, backfill, rollback; squawk output if it is installed, else by hand from the diff -->\n",
    },
    ShippedLabel {
        name: "security",
        kind: "area",
        human_merge: true,
        skills: &[("orqa-security-and-hardening", "addyosmani/agent-skills/skills/security-and-hardening")],
        guidance: "An item of security-and-hardening's Ask First tier (a new auth flow, a new PII category, CORS, upload handlers, rate limits) goes in the Plan under Open question.",
        review: Some(("orqa-security-review", "getsentry/skills/skills/security-review", true)),
        rows: &[],
        pr_section: "## Threat note\n<!-- the trust boundary touched; the Extra review Findings fixed and declined, with the reason -->\n",
    },
    ShippedLabel {
        name: "architecture",
        kind: "area",
        human_merge: false,
        skills: &[
            ("orqa-codebase-design", "mattpocock/skills/skills/engineering/codebase-design"),
            ("orqa-domain-modeling", "mattpocock/skills/skills/engineering/domain-modeling"),
        ],
        guidance: "Record a decision as an ADR and a new term in CONTEXT.md.",
        review: None,
        rows: &[],
        pr_section: "## Structure\n<!-- a Mermaid flowchart or sequenceDiagram of the new structure; the ADR if one was written -->\n",
    },
    ShippedLabel {
        name: "infra",
        kind: "area",
        human_merge: true,
        skills: &[
            ("orqa-terraform-style-guide", "hashicorp/agent-skills/plugins/terraform/skills/terraform-style-guide"),
            ("orqa-terraform-test", "hashicorp/agent-skills/plugins/terraform/skills/terraform-test"),
            ("orqa-docker-build-strategies", "docker/skills/skills/docker-build-strategies"),
            ("orqa-kubernetes-skill", "lukasniessen/kubernetes-skill"),
            ("orqa-ci-cd-and-automation", "addyosmani/agent-skills/skills/ci-cd-and-automation"),
            ("orqa-github-actions-hardening", "github/awesome-copilot/skills/github-actions-hardening"),
        ],
        guidance: "Validate with offline commands only: never terraform plan, apply, destroy, state, import, force-unlock or workspace, init with a backend, kubectl or helm against a cluster, docker push or login, gh secret, gh variable or gh workflow run, or anything that reads a secret; terraform test with mock_provider only. Anything needing a credential is filed as Manual work. Commit a .terraform.lock.hcl change only when the Ticket adds or upgrades a provider, regenerated with terraform providers lock for every platform the lock file lists; without network, file it as Manual work.",
        review: Some(("orqa-infra-review", "", false)),
        rows: &[],
        pr_section: "## Infra\n<!-- a Mermaid diagram of the resources touched; replacement risks; rollout and rollback; the offline checks' output; Manual work for plan and apply; never a plan -->\n",
    },
    ShippedLabel {
        name: "codex-review",
        kind: "modifier",
        human_merge: false,
        skills: &[],
        guidance: "",
        review: None,
        rows: &[("review", "app", "codex")],
        pr_section: "",
    },
];

/// The generic default PR template init writes: create-pr's five sections,
/// each with a one-line comment saying what goes there.
pub(crate) const PR_TEMPLATE: &str = "## What
<!-- one or two sentences, concrete -->

## Why
<!-- the Ticket or decision behind it -->

## Impact
<!-- what a reviewer should look at: migrations, public API or contract changes, security- or auth-adjacent paths, anything the checks do not cover -->

## Testing
<!-- the exact commands run and their result -->

## Ticket
<!-- the issue this closes -->
";

/// Where the label templates go.
pub(crate) const TEMPLATE_DIR: &str = ".github/PULL_REQUEST_TEMPLATE";
/// Where init writes the default template.
pub(crate) const DEFAULT_TEMPLATE: &str = ".github/pull_request_template.md";

/// The repo's default PR template, where GitHub reads one: .github, the
/// root or docs, named pull_request_template.md in any case.
pub(crate) fn default_template(repo: &Path) -> Option<PathBuf> {
    [".github", "", "docs"].into_iter().find_map(|dir| {
        fs::read_dir(repo.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .eq_ignore_ascii_case("pull_request_template.md")
                })
            })
    })
}

/// The .md files of a template directory, sorted; none for a missing one.
pub(crate) fn template_files(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md"))
        .collect();
    files.sort();
    files
}

/// A label's template: the default frame, a blank line, the label's section.
pub(crate) fn with_section(frame: &str, section: &str) -> String {
    let gap = if frame.ends_with('\n') { "\n" } else { "\n\n" };
    format!("{frame}{gap}{section}")
}

/// Writes the PR templates for the checked Area labels: the default at
/// .github/pull_request_template.md and TEMPLATE_DIR/<name>.md, the default
/// plus the label's section, for each label without one. Nothing existing
/// is overwritten, and nothing is asked when every file is there. A
/// default the repo has, wherever GitHub reads it, is asked to frame the
/// label templates (yes, or nobody answering; no writes nothing) and stays
/// where it is; with only a template directory, one of its files is
/// picked as the default and copied there, or none, which writes the
/// generic one.
fn write_pr_templates(
    repo: &Path,
    labels: &[&ShippedLabel],
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    let dir = repo.join(TEMPLATE_DIR);
    let missing: Vec<&ShippedLabel> = labels
        .iter()
        .copied()
        .filter(|label| !dir.join(label.template_file()).exists())
        .collect();
    let default = default_template(repo);
    if default.is_some() && missing.is_empty() {
        return Ok(());
    }
    step(out, named("PR TEMPLATES"))?;
    let frame = match default {
        Some(path) => {
            let name = path.strip_prefix(repo).unwrap_or(&path).display();
            let question =
                format!("Make {name} the default and add the label templates built from it?");
            if yes(out, input, tty, &question, true)? == Some(false) {
                return Ok(());
            }
            fs::read_to_string(&path)?
        }
        None => {
            let files = template_files(&dir);
            let picked = if files.is_empty() {
                None
            } else {
                let options: Vec<&str> = files.iter().map(String::as_str).chain(["none"]).collect();
                let none = files.len();
                let question = "Which of these is the default template?";
                raw(tty, || {
                    choose(out, &mut *input, question, &options, (none, none), "")
                })?
                .filter(|&i| i != none)
                .map(|i| fs::read_to_string(dir.join(&files[i])))
                .transpose()?
            };
            let frame = picked.unwrap_or_else(|| PR_TEMPLATE.to_string());
            let at = repo.join(DEFAULT_TEMPLATE);
            fs::create_dir_all(at.parent().unwrap())?;
            fs::write(&at, &frame)?;
            write!(out, "init: wrote {DEFAULT_TEMPLATE}\r\n")?;
            frame
        }
    };
    for label in missing {
        let name = label.template_file();
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(&name), with_section(&frame, label.pr_section))?;
        write!(out, "init: wrote {TEMPLATE_DIR}/{name}\r\n")?;
    }
    Ok(())
}

/// The shipped Ticket labels as a checklist, all checked: the user unchecks
/// the ones this repo will not use. Each checked label gets its entry in
/// config.json's labels, unless it has one already, which is never
/// overwritten, and its skills installed through the Skill manifest. An
/// unchecked label's entry, if any, is left too: init never deletes one.
/// Nobody answering keeps every label. Then the PR templates
/// (write_pr_templates), and each checked Area label whose template file
/// is there gets it as its pr_template, unless the entry names one.
fn ask_labels(
    repo: &Path,
    tools: &dyn Tools,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<()> {
    step(out, named("TICKET LABELS"))?;
    note(
        out,
        "an orqa:<name> bd label on a Ticket loads the label's skills and guidance into the Stages that write its code",
    )?;
    let rows: Vec<(&str, String)> = LABELS
        .iter()
        .map(|label| {
            let skills: Vec<&str> = label
                .skills
                .iter()
                .map(|(name, _)| name.strip_prefix(manifest::PREFIX).unwrap_or(name))
                .collect();
            (label.name, skills.join(", "))
        })
        .collect();
    let question = "Which Ticket labels will this repo use?";
    let all = vec![true; rows.len()];
    let checked = raw(tty, || checklist(out, input, question, &rows, &all))?;
    let (path, mut doc) = app::read_object(repo).map_err(io::Error::other)?;
    if !matches!(doc["labels"], Value::Null | Value::Object(_)) {
        return Err(io::Error::other("config.json's labels is not an object"));
    }
    let picked = || LABELS.iter().zip(&checked).filter(|(_, on)| **on);
    for (label, _) in picked() {
        if doc["labels"][label.name].is_null() {
            doc["labels"][label.name] = label.entry();
        }
    }
    let areas: Vec<&ShippedLabel> = picked()
        .map(|(label, _)| label)
        .filter(|label| label.kind == "area")
        .collect();
    write_pr_templates(repo, &areas, out, input, tty)?;
    // The mapping follows the file, written now or kept: an entry's own
    // non-empty pr_template stays, and a broken entry is not init's to fix.
    for label in areas {
        let file = label.template_file();
        let entry = &mut doc["labels"][label.name];
        let unset = entry["pr_template"].as_str().unwrap_or_default().is_empty();
        if entry.is_object() && unset && repo.join(TEMPLATE_DIR).join(&file).exists() {
            entry["pr_template"] = json!(file);
        }
    }
    app::write(&path, &doc).map_err(io::Error::other)?;
    let wanted = picked().flat_map(|(label, _)| {
        label
            .sources()
            .map(move |skill| (skill, format!("for orqa:{}", label.name)))
    });
    install_missing(repo, tools, out, wanted)
}

/// TypeSafe's opt-in, kept on or off in config.json. TYPESAFE_API_KEY
/// (`env_key`) set is on, unasked. Otherwise yes takes the key stored, or
/// asks for one; no, or an empty key, is off. Nobody answering (a
/// non-interactive init) leaves it on only where it is on with a key kept.
/// The key stays in its own file.
pub(crate) fn ask_typesafe(
    repo: &Path,
    env_key: &str,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<bool> {
    let question = "Use TypeSafe to judge plans, Wakes and disputed Findings?";
    let kept = repo.join(KEY_FILE).exists();
    let on = if !env_key.trim().is_empty() {
        true
    } else {
        step(out, brand::chip("typesafe"))?;
        match yes(out, input, tty, question, true)? {
            Some(true) => kept || ask_typesafe_key(repo, out, input, tty)?,
            Some(false) => false,
            None => kept && app::typesafe(repo),
        }
    };
    app::set_typesafe(repo, on).map_err(io::Error::other)?;
    write!(out, "init: TypeSafe {}\r\n", if on { "on" } else { "off" })?;
    Ok(on)
}

/// Asks for the TypeSafe API key with echo off and keeps it, readable only
/// by the user: whether one was kept. An empty answer, Ctrl-C, Ctrl-D or a
/// silent stdin keeps none.
fn ask_typesafe_key(
    repo: &Path,
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
) -> io::Result<bool> {
    let what = format!("TypeSafe API key, kept in {KEY_FILE}");
    let key = secret(out, input, tty, "key", &what)?;
    if key.is_empty() {
        return Ok(false);
    }
    keep_key(repo, &key)?;
    write!(out, "init: TypeSafe key kept in {KEY_FILE}\r\n")?;
    Ok(true)
}

/// Keeps the TypeSafe key in KEY_FILE, readable only by the user.
pub(crate) fn keep_key(repo: &Path, key: &str) -> io::Result<()> {
    local_dir(repo)?;
    File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(repo.join(KEY_FILE))?
        .write_all(format!("{key}\n").as_bytes())
}

/// Puts a yes/no question as a menu, `default` selected: y or n selects,
/// enter answers, Ctrl-C or Ctrl-D is no. None when the input ends
/// unanswered: a non-interactive init. A key selects, never answers, so the
/// enter typed after a y never answers the next question.
fn yes(
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
    question: &str,
    default: bool,
) -> io::Result<Option<bool>> {
    let options = ["Yes", "No"];
    let picked = raw(tty, || {
        choose(
            out,
            input,
            question,
            &options,
            (usize::from(!default), 1),
            "yn",
        )
    })?;
    Ok(picked.map(|i| i == 0))
}

/// Asks for a secret, `what` above a `prompt ›` line that shows a bullet
/// per character, so a paste shows it landed: the text, or "" on enter
/// alone, Ctrl-C, Ctrl-D or a silent stdin.
fn secret(
    out: &mut dyn Write,
    input: &mut dyn Read,
    tty: bool,
    prompt: &str,
    what: &str,
) -> io::Result<String> {
    note(out, &format!("{what} · enter to skip"))?;
    let prompt = Span::styled(format!("  {prompt} › "), PURPLE).bold();
    write!(out, "{}", paint(vec![prompt]))?;
    out.flush()?;
    let text = match raw(tty, || read_line(out, input))? {
        Line::Text(text) => text,
        Line::Cancel | Line::End => String::new(),
    };
    write!(out, "\r\n")?;
    Ok(text)
}

/// What read_line read.
enum Line {
    Text(String),
    /// Ctrl-C or Ctrl-D.
    Cancel,
    /// The input ended with nothing typed: nobody is there to answer.
    End,
}

/// Reads a line, echoing a bullet for each character.
fn read_line(out: &mut dyn Write, input: &mut dyn Read) -> io::Result<Line> {
    let text = |line: &[u8]| Line::Text(String::from_utf8_lossy(line).trim().to_string());
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while let Ok(1) = input.read(&mut byte) {
        match byte[0] {
            b'\r' | b'\n' => return Ok(text(&line)),
            3 | 4 => return Ok(Line::Cancel),
            0x7f | 0x08 => {
                if line.pop().is_some() {
                    write!(out, "\x08 \x08")?;
                }
            }
            0x1b => {
                // An escape sequence, an arrow key say: skipped whole.
                if let Ok(1) = input.read(&mut byte) {
                    if byte[0] == b'[' {
                        while let Ok(1) = input.read(&mut byte) {
                            if (0x40..=0x7e).contains(&byte[0]) {
                                break;
                            }
                        }
                    }
                }
            }
            b if b < 0x20 => {}
            b => {
                line.push(b);
                write!(out, "{}", paint(vec![Span::styled("•", TEXT)]))?;
            }
        }
        out.flush()?;
    }
    Ok(if line.is_empty() {
        Line::End
    } else {
        text(&line)
    })
}

/// Colors 24-bit when COLORTERM says so, else the 256 cube, as the Shell does.
fn truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor" | "24bit")
    )
}

/// Spans as ANSI text, in the Shell's palette.
fn paint(spans: Vec<Span<'_>>) -> String {
    brand::ansi(&Row::from(spans), truecolor())
}

/// The width init draws its rules and banner to: the terminal's, up to 100.
fn width() -> usize {
    match crossterm::terminal::size() {
        Ok((w, _)) if w > 0 => usize::from(w).min(100),
        _ => 80, // not a terminal, or one that reports no size
    }
}

/// A step's own name in its rule, where no provider's chip stands for it.
pub(crate) fn named(name: &str) -> Span<'static> {
    Span::styled(name.to_string(), MUTED)
}

/// Opens a step with a rule, as the Shell's RECENT has: `── label ────`.
pub(crate) fn step(out: &mut dyn Write, label: Span<'static>) -> io::Result<()> {
    let fill = width().saturating_sub(label.content.chars().count() + 4);
    let rule = vec![
        Span::styled("── ", BORDER),
        label,
        Span::styled(format!(" {}", "─".repeat(fill)), BORDER),
    ];
    write!(out, "\r\n{}\r\n", paint(rule))
}

/// A muted line under a step's rule.
fn note(out: &mut dyn Write, text: &str) -> io::Result<()> {
    write!(
        out,
        "{}\r\n",
        paint(vec![Span::styled(format!("  {text}"), MUTED)])
    )
}

/// Orqadence's header, as the Shell draws it: the pane mark and the
/// wordmark (the name, where it does not fit) in a rounded box with the
/// version on its border, then the folder being set up. Nothing on a
/// terminal too narrow for the name.
pub(crate) fn banner(out: &mut dyn Write, folder: &str) -> io::Result<()> {
    let w = width();
    if w < 40 {
        return Ok(());
    }
    let lit = brand::PANE_COLORS[brand::REST];
    let mut rows = vec![vec![Span::styled(
        format!("╭{}╮", "─".repeat(w - 2)),
        FRAME,
    )]];
    for (i, mark) in brand::logo_mark(brand::REST).into_iter().enumerate() {
        let mut row = vec![Span::styled("│  ", FRAME)];
        row.extend(mark.spans);
        row.push("   ".into());
        if w >= 88 {
            row.push(Span::styled(brand::WORDMARK_ROWS[i], brand::WORDMARK));
            row.push(" ".into());
            row.push(Span::styled(brand::CURSOR_ROWS[i], lit));
        } else if i == 2 {
            row.push(Span::styled("Orqadence ", brand::WORDMARK).bold());
            row.push(Span::styled("▁▁", lit));
        }
        let used: usize = row.iter().map(|s| s.content.chars().count()).sum();
        let pad = " ".repeat(w.saturating_sub(used + 1));
        row.push(Span::styled(format!("{pad}│"), FRAME));
        rows.push(row);
    }
    let version = format!(" {} ", crate::version::version());
    let fill = "─".repeat(w.saturating_sub(version.chars().count() + 3));
    rows.push(vec![
        Span::styled(format!("╰{fill}"), FRAME),
        Span::styled(version, brand::PANE_COLORS[3]),
        Span::styled("─╯", FRAME),
    ]);
    rows.push(vec![
        Span::styled("  orqa init", TEXT),
        Span::styled(" · setting up ", MUTED),
        Span::styled(folder.to_string(), YELLOW),
    ]);
    for row in rows {
        write!(out, "{}\r\n", paint(row))?;
    }
    Ok(())
}

/// The TypeSafe key for a Judgment: TYPESAFE_API_KEY when set, else the key
/// init kept in the Target repo, else None.
pub(crate) fn typesafe_key(repo: &Path, env: &dyn Fn(&str) -> String) -> Option<String> {
    let from_env = env("TYPESAFE_API_KEY");
    let key = if from_env.trim().is_empty() {
        fs::read_to_string(repo.join(KEY_FILE)).unwrap_or_default()
    } else {
        from_env
    };
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

/// Runs `f` with the terminal in raw mode when stdin is one, so keys arrive
/// one at a time and nothing typed is echoed.
fn raw<T>(tty: bool, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let raw = tty && crossterm::terminal::enable_raw_mode().is_ok();
    let result = f();
    if raw {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    result
}

/// Puts `question` over a numbered menu as the Shell's QUESTION box does,
/// `default` of `(default, cancel)` under the cursor: the arrow keys (or
/// j/k) move it, the nth of `hotkeys` moves it to the nth option, enter
/// submits, a digit picks its option outright, and Ctrl-C, Ctrl-D or q pick
/// `cancel`. Answered, the menu folds into a line naming the choice. None
/// when the input ends before any key: nobody is there to answer; a key and
/// then the end leaves the cursor's. Reads a byte at a time, so a scripted
/// "y\n" is two keys.
fn choose(
    out: &mut dyn Write,
    input: &mut dyn Read,
    question: &str,
    options: &[&str],
    (default, cancel): (usize, usize),
    hotkeys: &str,
) -> io::Result<Option<usize>> {
    write!(
        out,
        "{}\r\n",
        paint(vec![Span::styled(format!("  {question}"), TEXT).bold()])
    )?;
    let hint = format!("  ↑↓ move · 1-{} pick · enter choose", options.len());
    let draw = |out: &mut dyn Write, sel: usize| -> io::Result<()> {
        for (i, option) in options.iter().enumerate() {
            let (mark, style) = if i == sel {
                ("›", Style::new().fg(PURPLE).bold())
            } else {
                (" ", Style::new().fg(TEXT))
            };
            let row = Span::styled(format!("  {mark} {}. {option}", i + 1), style);
            write!(out, "{}\x1b[K\r\n", paint(vec![row]))?;
        }
        write!(
            out,
            "{}\x1b[K\r",
            paint(vec![Span::styled(hint.as_str(), MUTED)])
        )?;
        out.flush()
    };
    let (mut sel, mut pressed) = (default, false);
    draw(out, sel)?;
    while let Some(key) = next_key(input) {
        pressed = true;
        match key {
            0xc1 | 0xc4 | b'k' => sel = sel.saturating_sub(1), // up, left
            0xc2 | 0xc3 | b'j' => sel = (sel + 1).min(options.len() - 1), // down, right
            b'\r' | b'\n' => return done(out, options, sel),
            3 | 4 | b'q' => return done(out, options, cancel),
            digit if digit > b'0' && usize::from(digit - b'0') <= options.len() => {
                return done(out, options, usize::from(digit - b'0') - 1)
            }
            key => {
                let key = char::from(key).to_ascii_lowercase();
                if let Some(i) = hotkeys.chars().position(|hot| hot == key) {
                    sel = i;
                }
            }
        }
        write!(out, "\x1b[{}A", options.len())?; // back over the menu and redraw it
        draw(out, sel)?;
    }
    if pressed {
        return done(out, options, sel);
    }
    write!(out, "\r\n")?;
    Ok(None)
}

fn next_byte(input: &mut dyn Read) -> Option<u8> {
    let mut byte = [0u8; 1];
    matches!(input.read(&mut byte), Ok(1)).then_some(byte[0])
}

/// The next key: a byte, or an arrow key (ESC [ A to D) flagged 0x80;
/// any other escape sequence is skipped whole and reads as 0.
fn next_key(input: &mut dyn Read) -> Option<u8> {
    Some(match next_byte(input)? {
        0x1b if next_byte(input) == Some(b'[') => loop {
            match next_byte(input) {
                Some(b @ 0x40..=0x7e) => break b | 0x80,
                Some(_) => {}
                None => break 0,
            }
        },
        key => key,
    })
}

/// Puts `question` over a checklist of (name, detail) options, checked as
/// `default` says: the arrow keys (or j/k) move the cursor, space toggles
/// its option, a digit toggles the nth, and enter submits. Ctrl-C, Ctrl-D
/// or q keep the default, as does the input ending before any key; a key
/// and then the end submits what is checked. Answered, the list folds into
/// a line naming the checked options.
fn checklist(
    out: &mut dyn Write,
    input: &mut dyn Read,
    question: &str,
    options: &[(&str, String)],
    default: &[bool],
) -> io::Result<Vec<bool>> {
    write!(
        out,
        "{}\r\n",
        paint(vec![Span::styled(format!("  {question}"), TEXT).bold()])
    )?;
    let hint = format!(
        "  ↑↓ move · space or 1-{} toggle · enter choose",
        options.len()
    );
    let width = options
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0)
        + 2;
    let draw = |out: &mut dyn Write, sel: usize, on: &[bool]| -> io::Result<()> {
        for (i, (name, detail)) in options.iter().enumerate() {
            let (mark, style) = if i == sel {
                ("›", Style::new().fg(PURPLE).bold())
            } else {
                (" ", Style::new().fg(TEXT))
            };
            let tick = if on[i] { "x" } else { " " };
            let row = vec![
                Span::styled(
                    format!("  {mark} [{tick}] {}. {name:<width$}", i + 1),
                    style,
                ),
                Span::styled(detail.clone(), MUTED),
            ];
            write!(out, "{}\x1b[K\r\n", paint(row))?;
        }
        write!(
            out,
            "{}\x1b[K\r",
            paint(vec![Span::styled(hint.as_str(), MUTED)])
        )?;
        out.flush()
    };
    let (mut sel, mut on) = (0, default.to_vec());
    // Auto-wrap off, so a row wider than the terminal stays one row and the
    // moves up count right; fold turns it back on.
    write!(out, "\x1b[?7l")?;
    draw(out, sel, &on)?;
    while let Some(key) = next_key(input) {
        match key {
            0xc1 | 0xc4 | b'k' => sel = sel.saturating_sub(1), // up, left
            0xc2 | 0xc3 | b'j' => sel = (sel + 1).min(options.len() - 1), // down, right
            b'\r' | b'\n' => break,
            3 | 4 | b'q' => {
                on = default.to_vec();
                break;
            }
            b' ' => on[sel] = !on[sel],
            // ponytail: one digit each, as choose has; letters past 9 if a list ever grows.
            digit if digit > b'0' && usize::from(digit - b'0') <= options.len() => {
                on[usize::from(digit - b'0') - 1] ^= true;
            }
            _ => {}
        }
        write!(out, "\x1b[{}A", options.len())?; // back over the list and redraw it
        draw(out, sel, &on)?;
    }
    let said = options
        .iter()
        .zip(&on)
        .filter(|(_, on)| **on)
        .map(|((name, _), _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    fold(
        out,
        options.len(),
        if said.is_empty() { "none" } else { &said },
    )?;
    Ok(on)
}

/// Folds the menu into one line naming the choice.
fn done(out: &mut dyn Write, options: &[&str], sel: usize) -> io::Result<Option<usize>> {
    fold(out, options.len(), options[sel])?;
    Ok(Some(sel))
}

/// Goes back up `lines` rows, clears them and what is below, and says the
/// answer in one line; auto-wrap back on, as the checklist turns it off.
fn fold(out: &mut dyn Write, lines: usize, said: &str) -> io::Result<()> {
    let answer = Span::styled(format!("  ✓ {said}"), GREEN);
    write!(
        out,
        "\x1b[{lines}A\r\x1b[J\x1b[?7h{}\r\n",
        paint(vec![answer])
    )?;
    out.flush()
}

/// Takes out of the file at path each line that is one of lines, trimmed,
/// and says whether any was there. A file that cannot be read has none.
pub(crate) fn remove_lines(path: &Path, lines: &[String]) -> io::Result<bool> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(false);
    };
    let kept: String = text
        .split_inclusive('\n')
        .filter(|line| !lines.iter().any(|drop| line.trim() == drop))
        .collect();
    if kept == text {
        return Ok(false);
    }
    fs::write(path, kept)?;
    Ok(true)
}

/// Returns one specific message per missing prerequisite.
pub(crate) fn preflight(
    repo: &Path,
    tools: &dyn Tools,
    env: &dyn Fn(&str) -> String,
) -> Vec<String> {
    let mut missing = Vec::new();
    if !repo.join(".beads").exists() {
        missing.push("no bd workspace here: run 'bd init'".to_string());
    }
    if tools.run(repo, &["gh", "auth", "status"]).is_err() {
        missing.push("gh is not authenticated: run 'gh auth login'".to_string());
    }
    if !tools
        .run(repo, &["git", "remote"])
        .is_ok_and(|remotes| !remotes.trim().is_empty())
    {
        missing.push("no git remote: add one with 'git remote add origin <url>'".to_string());
    }
    let home = PathBuf::from(env("HOME"));
    let found = manifest::list(repo, &home, tools, manifest::personal(repo));
    if !found.iter().any(|(name, _)| name == CREATE_PR) {
        missing.push(format!(
            "no {CREATE_PR} skill: run 'orqa init' to install the shipped one"
        ));
    }
    // Each job's pick, but none, as the App its row runs on loads or has
    // built in one.
    match Manifest::load(repo) {
        Ok(manifest) => {
            for (job, suggestions) in JOBS {
                let pick = manifest.pick(job);
                // A row that cannot be read is the Orchestrator's to refuse.
                let Ok(row) = app::row(repo, manifest::job_row(job), &[]) else {
                    continue;
                };
                let have: Vec<String> = found
                    .iter()
                    .filter(|(name, path)| {
                        path.parent().is_some_and(|dir| row.app.loads(name, dir))
                    })
                    .map(|(name, _)| name.clone())
                    .collect();
                if manifest::lacks(pick, &have, row.app.built_in) {
                    // init installs only the default
                    let fix = if pick == suggestions[0].0 {
                        "orqa init installs it, or /config picks another"
                    } else {
                        "/config installs it, or picks another"
                    };
                    missing.push(format!(
                        "the {} skill {pick} is missing: {fix}",
                        job.replace('-', " ")
                    ));
                }
            }
        }
        Err(err) => missing.push(err),
    }
    // Each configured label's skills and Extra review skill, which init's
    // install may have failed to fetch; a broken entry is app::checks'.
    let labels = app::read(repo)
        .map(|(_, doc)| app::labels(&doc))
        .unwrap_or_default();
    for (name, label) in labels {
        let Ok(label) = label else {
            continue;
        };
        let review = Some(label.extra_review.skill).filter(|skill| !skill.is_empty());
        for skill in label.skills.into_iter().chain(review) {
            if !found.iter().any(|(have, _)| *have == skill) {
                missing.push(format!(
                    "orqa:{name}'s skill {skill} is missing: orqa init installs a shipped label's"
                ));
            }
        }
    }
    // A row that cannot be read is the Orchestrator's to refuse; the
    // Review's fallback, unset, runs nothing.
    for key in app::ROWS {
        let row = match key {
            app::IF_LIMITED => app::fallback_row(repo, &[]).ok().flatten(),
            _ => app::row(repo, key, &[]).ok(),
        };
        let Some(row) = row else {
            continue;
        };
        let name = row.app.bin;
        if tools.run(repo, &["which", name]).is_err() {
            missing.push(format!("{key} runs on {name}, which is not on PATH"));
        }
    }
    if has_label(repo, "infra") {
        for tool in INFRA_TOOLS {
            if tools.run(repo, &["which", tool]).is_err() {
                missing.push(format!(
                    "orqa:infra's Extra review needs {tool}, which is not on PATH"
                ));
            } else if tool == "terraform" {
                let version = tools
                    .run(repo, &["terraform", "version", "-json"])
                    .ok()
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                    .and_then(|doc| major_minor(doc["terraform_version"].as_str()?));
                if !version.is_some_and(|found| found >= (1, 7)) {
                    missing.push(
                        "orqa:infra's Extra review needs terraform 1.7 or newer (for mock_provider)"
                            .to_string(),
                    );
                }
            }
        }
    }
    if env("HERDR_ENV") != "1" {
        missing.push("HERDR_ENV is not 1: run Orqadence from a pane inside herdr".to_string());
    }
    missing
}

/// The tools orqa:infra's Extra review runs (infra-review's checks), every
/// one wanted whether or not the repo uses it.
const INFRA_TOOLS: [&str; 8] = [
    "terraform",
    "tflint",
    "trivy",
    "hadolint",
    "helm",
    "kubeconform",
    "actionlint",
    "shellcheck",
];

/// Whether config.json's labels has an entry under name, readable or not:
/// a broken one is app::checks' to report.
fn has_label(repo: &Path, name: &str) -> bool {
    app::read(repo).is_ok_and(|(_, doc)| !doc["labels"][name].is_null())
}

/// 1.2.3 as (1, 2), the patch dropped; anything else is None.
fn major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.splitn(3, '.');
    let mut next = || parts.next()?.parse().ok();
    Some((next()?, next()?))
}

/// What the preflight warns of without failing: the superpowers plugin, an
/// installed Stage skill that lost a job's placeholder, which the shipped
/// one holds, and, with orqa:fe configured, a gh too old to attach its
/// screenshots. A personal skill shadowing a committed one is a Question when
/// a Ticket starts (ask_shadowed).
pub(crate) fn warnings(repo: &Path, tools: &dyn Tools) -> Vec<String> {
    let mut warn = Vec::new();
    for (name, shipped) in SKILLS {
        let Some(Ok(installed)) = stage_skill(repo, name) else {
            continue;
        };
        for (job, _) in JOBS {
            let held = manifest::placeholder(job);
            if shipped.contains(&held) && !installed.contains(&held) {
                warn.push(format!(
                    "the installed {name} lacks {held}: the {} skill you pick never runs there; put the line back, or refresh it with orqa init",
                    job.replace('-', " ")
                ));
            }
        }
    }
    if manifest::plugins(repo, tools)
        .iter()
        .any(|(name, _)| name == "superpowers")
    {
        warn.push(
            "the superpowers Claude Code plugin is enabled: its SessionStart hook can stall Stages"
                .to_string(),
        );
    }
    // An unreadable gh version says nothing: preflight's gh auth status blocks.
    if has_label(repo, "fe") {
        if let Ok(text) = tools.run(repo, &["gh", "--version"]) {
            let version = text.split_whitespace().nth(2).unwrap_or_default();
            if major_minor(version).is_some_and(|found| found < (2, 99)) {
                warn.push(format!(
                    "orqa:fe is configured, and gh {version} cannot attach screenshots to the pull request: gh --attach needs 2.99 (github.com or Enterprise Cloud); upgrade gh"
                ));
            }
        }
    }
    warn
}

/// Prints each missing prerequisite and returns the exit code.
pub(crate) fn report_missing(out: &mut dyn Write, missing: &[String]) -> i32 {
    for m in missing {
        let _ = writeln!(out, "preflight: {m}");
    }
    if missing.is_empty() {
        let ready = Span::styled("  ✓ ready: run orqa to open the Shell", GREEN);
        let _ = writeln!(out, "{}", paint(vec![ready]));
    }
    i32::from(!missing.is_empty())
}

#[cfg(test)]
pub(crate) mod setup_test;
