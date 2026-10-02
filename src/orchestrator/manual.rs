//! Manual work: an item a Stage's session filed for the user, one folder
//! `manual-work/<n>/` in its Run directory: manual-work.md always, and
//! wizard.sh or prompt.md as the task needs. The session writes every file;
//! this reads them, and marks an item done.

use std::fs;
use std::path::{Path, PathBuf};

use crate::tools::Tools;

/// One item, as its folder holds it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub(crate) folder: PathBuf,
    /// Its sections, as written; empty when one is not there.
    pub(crate) what: String,
    pub(crate) why: String,
    pub(crate) how: String,
    /// Its session waits on it: `Blocks: yes` on manual-work.md's first line.
    pub(crate) blocks: bool,
}

/// Reads the item `filed` names, relative to `run_dir` or absolute: an
/// error when it is not one of run_dir's manual-work/<n>/ folders or its
/// manual-work.md is missing.
pub(crate) fn read(run_dir: &Path, filed: &Path) -> Result<Item, String> {
    let folder = run_dir.join(filed);
    // canonical, so neither ../ nor a link reaches another Ticket's item:
    // manual-work itself linked elsewhere is not run_dir's
    let real = folder.canonicalize().unwrap_or_default();
    let items = run_dir
        .canonicalize()
        .ok()
        .map(|dir| dir.join("manual-work"));
    if real.parent() != items.as_deref() || number(&real).parse::<u64>().is_err() {
        return Err(format!(
            "{} is not a folder manual-work/<n>/ in {}",
            folder.display(),
            run_dir.display()
        ));
    }
    let file = folder.join("manual-work.md");
    let body = fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
    let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in body.lines() {
        match (line.strip_prefix("## "), sections.last_mut()) {
            (Some(heading), _) => sections.push((heading.trim(), Vec::new())),
            (None, Some((_, text))) => text.push(line),
            (None, None) => {}
        }
    }
    let section = |name: &str| {
        sections
            .iter()
            .find(|(heading, _)| heading.eq_ignore_ascii_case(name))
            .map_or(String::new(), |(_, text)| {
                text.join("\n").trim().to_string()
            })
    };
    Ok(Item {
        what: section("What"),
        why: section("Why"),
        how: section("How"),
        blocks: body.lines().next().is_some_and(|first| {
            first
                .split('·')
                .any(|field| field.trim().eq_ignore_ascii_case("Blocks: yes"))
        }),
        folder,
    })
}

/// The items open in `run_dir`, its manual-work/<n>/ folders read, in
/// number order; one that does not read is left out, and so is a link, which
/// would list its target's item twice.
pub(crate) fn open(run_dir: &Path) -> Vec<Item> {
    let entries = fs::read_dir(run_dir.join("manual-work"))
        .into_iter()
        .flatten();
    let mut items: Vec<Item> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if entry.file_type().ok()?.is_symlink() {
                return None;
            }
            read(run_dir, &entry.path()).ok()
        })
        .collect();
    items.sort_by_key(|item| number(&item.folder).parse::<u64>().unwrap_or_default());
    items
}

/// The Manual work Input: a line per open item, its folder, Blocks and
/// What, or none. One with no What yet is still being written, listed once
/// it has one.
pub(crate) fn input(run_dir: &Path) -> String {
    let lines: Vec<String> = open(run_dir)
        .iter()
        .filter(|item| !item.what.is_empty())
        .map(|item| {
            let blocks = if item.blocks { "yes" } else { "no" };
            format!(
                "\n  {} · Blocks: {blocks} · {}",
                item.folder.display(),
                item.what
            )
        })
        .collect();
    match lines.is_empty() {
        true => "none".to_string(),
        false => lines.concat(),
    }
}

/// An item's number, its folder's name: the n of manual-work/<n>/.
pub(crate) fn number(folder: &Path) -> String {
    folder
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// The prompt that tells the waiting session its item is done, with the
/// user's Report back `facts`, empty for Done alone.
pub(crate) fn done_prompt(folder: &Path, facts: &str) -> String {
    format!("Manual work {} done: {facts}", number(folder))
}

/// Marks `item` done: a bd comment on `id`, whose Run directory holds it,
/// with its What and the user's `facts`, then its folder deleted. A comment
/// bd refuses leaves the folder.
pub(crate) fn done(
    tools: &dyn Tools,
    repo: &Path,
    id: &str,
    item: &Item,
    facts: &str,
) -> Result<(), String> {
    let mut comment = done_prompt(&item.folder, &item.what);
    if !facts.is_empty() {
        comment += &format!("\n\nFacts: {facts}");
    }
    tools
        .run(repo, &["bd", "comments", "add", id, &comment])
        .map_err(|err| err.to_string())?;
    fs::remove_dir_all(&item.folder).map_err(|err| format!("{}: {err}", item.folder.display()))
}
