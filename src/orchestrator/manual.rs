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
}

/// Reads the item `filed` names, relative to `run_dir` or absolute: an
/// error when it is not one of run_dir's manual-work/<n>/ folders, its
/// manual-work.md is missing, or that file's first line is not Ticket,
/// Stage and Blocks.
pub(crate) fn read(run_dir: &Path, filed: &Path) -> Result<Item, String> {
    let folder = run_dir.join(filed);
    // canonical, so neither ../ nor a link reaches another Ticket's item
    let real = folder.canonicalize().unwrap_or_default();
    let items = run_dir.join("manual-work").canonicalize().ok();
    if real.parent().is_none()
        || real.parent() != items.as_deref()
        || number(&real).parse::<u64>().is_err()
    {
        return Err(format!(
            "{} is not a folder manual-work/<n>/ in {}",
            folder.display(),
            run_dir.display()
        ));
    }
    let file = folder.join("manual-work.md");
    let body = fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default();
    let field = |name: &str| {
        first.split(" · ").any(|part| {
            let part = part.trim().strip_prefix(name);
            part.is_some_and(|rest| rest.starts_with(':'))
        })
    };
    if !["Ticket", "Stage", "Blocks"].into_iter().all(field) {
        return Err(format!(
            "{}: its first line is not Ticket: <id> · Stage: <stage> · Blocks: yes|no",
            file.display()
        ));
    }
    let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in lines {
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
        folder,
    })
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
    let n = number(&item.folder);
    let mut comment = format!("Manual work {n} done: {}", item.what);
    if !facts.is_empty() {
        comment += &format!("\n\nFacts: {facts}");
    }
    tools
        .run(repo, &["bd", "comments", "add", id, &comment])
        .map_err(|err| err.to_string())?;
    fs::remove_dir_all(&item.folder).map_err(|err| format!("{}: {err}", item.folder.display()))
}
