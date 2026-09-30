//! The Skill manifest: the skills Orqadence installed for one checkout, and
//! each job's pick. Also fetching a third-party skill with git, and listing
//! the skills the user already has.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::iter;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use crate::on_call;
use crate::skills::CREATE_PR;
use crate::tempdir::TempDir;
use crate::tools::Tools;

const MANIFEST: &str = ".orqadence/skills.json";

/// Before the name of every skill Orqadence installs, its folder's and its
/// SKILL.md's: none shares a name with a skill of the user's own, which
/// claude would run instead.
pub(crate) const PREFIX: &str = "orqa-";

/// The per-person switch, in on_call::CONFIG, that lets the jobs pick and
/// the Stages load the user's personal skills: those at home and the
/// Claude Code plugins'. Off, only Orqadence's, the repo's own and the
/// Apps' built-in ones.
const PERSONAL: &str = "personal_skills";

/// The pick that opts a job out: its Stage skill follows its own instructions.
pub(crate) const NONE: &str = "none";

/// Each job a Delegate skill can do, with its suggestions, the default first:
/// (skill name, source). A skill from a source is named as installed, with
/// PREFIX; an empty source is built into an App (its built_in): nothing to
/// install.
pub(crate) const JOBS: &[(&str, &[(&str, &str)])] = &[
    (
        "test-first",
        &[
            ("orqa-tdd", "mattpocock/skills/skills/engineering/tdd"),
            (
                "orqa-test-driven-development",
                "obra/superpowers/skills/test-driven-development",
            ),
            (
                "orqa-test-driven-development",
                "addyosmani/agent-skills/skills/test-driven-development",
            ),
        ],
    ),
    (
        "self-review",
        &[
            (
                "orqa-code-review",
                "mattpocock/skills/skills/engineering/code-review",
            ),
            (
                "orqa-requesting-code-review",
                "obra/superpowers/skills/requesting-code-review",
            ),
        ],
    ),
    (
        "working-mode",
        &[
            ("orqa-ponytail", "DietrichGebert/ponytail/skills/ponytail"),
            (
                "orqa-karpathy-guidelines",
                "multica-ai/andrej-karpathy-skills/skills/karpathy-guidelines",
            ),
        ],
    ),
    (
        "prose",
        &[
            ("orqa-caveman", "JuliusBrussee/caveman/skills/caveman"),
            (
                "orqa-caveman-commit",
                "JuliusBrussee/caveman/skills/caveman-commit",
            ),
        ],
    ),
    (
        "review",
        &[
            (NONE, ""),
            ("review-agent", ""), // Codex's own
            (
                "orqa-requesting-code-review",
                "obra/superpowers/skills/requesting-code-review",
            ),
        ],
    ),
    (
        "audit",
        &[(
            "orqa-ponytail-review",
            "DietrichGebert/ponytail/skills/ponytail-review",
        )],
    ),
    (
        "merge-conflicts",
        &[
            (
                "orqa-resolving-merge-conflicts",
                "mattpocock/skills/skills/engineering/resolving-merge-conflicts",
            ),
            (
                "orqa-resolve-merge-conflicts",
                "warpdotdev/common-skills/.agents/skills/resolve-merge-conflicts",
            ),
        ],
    ),
];

/// A job's placeholder in a Stage skill: {{job}}, alone on the line that uses
/// the job's Delegate skill.
pub(crate) fn placeholder(job: &str) -> String {
    format!("{{{{{job}}}}}")
}

/// Whether a job's pick is a skill missing from have (list's names): none
/// needs nothing, nor does one built_in to the App running the job's line.
pub(crate) fn lacks(pick: &str, have: &[String], built_in: &[&str]) -> bool {
    pick != NONE && !built_in.contains(&pick) && !have.iter().any(|name| name == pick)
}

/// The config.json row whose App runs a job's line: the audit is side A's.
pub(crate) fn job_row(job: &str) -> &'static str {
    match job {
        "review" => "review",
        "audit" => "side_a",
        "merge-conflicts" => "address",
        _ => "implement",
    }
}

/// Where Orqadence's skills are, the one place (ADR 0006), from the
/// checkout's root: their folders in .orqadence/skills, committed, reached
/// through committed relative folder links from .agents/skills and
/// .claude/skills. The folder is linked, never its SKILL.md: codex skips a
/// SKILL.md that is itself a link.
pub(crate) const FILES: &str = ".orqadence/skills";
pub(crate) const LINKS: [&str; 2] = [".agents/skills", ".claude/skills"];

pub(crate) fn skill_dir(repo: &Path, name: &str) -> PathBuf {
    repo.join(FILES).join(name)
}

pub(crate) fn links(repo: &Path, name: &str) -> [PathBuf; 2] {
    LINKS.map(|dir| repo.join(dir).join(name))
}

/// What each link points at: both links' folders are two deep.
pub(crate) fn target(name: &str) -> PathBuf {
    Path::new("../..").join(FILES).join(name)
}

/// The first of its folders Orqadence may not write to (own).
fn unowned(repo: &Path) -> Option<&'static str> {
    iter::once(FILES).chain(LINKS).find(|dir| !own(repo, dir))
}

/// One skill Orqadence installed, in .orqadence/skills.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Installed {
    /// The clone URL; empty for a Shipped skill.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) repo: String,
    #[serde(rename = "ref", skip_serializing_if = "String::is_empty")]
    pub(crate) git_ref: String,
    /// The skill's folder in the repo.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) commit: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) shipped: bool,
}

/// The Skill manifest, kept in .orqadence/ beside init's record of the text it
/// wrote.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Manifest {
    pub(crate) skills: BTreeMap<String, Installed>,
    /// Job to its pick: a skill name or "none". A job left out takes its
    /// default.
    pub(crate) picks: BTreeMap<String, String>,
}

impl Manifest {
    /// A checkout without one has an empty manifest. A garbled one is an
    /// error, so that saving over it cannot lose what it held. Errors are
    /// messages for the Shell.
    pub(crate) fn load(repo: &Path) -> Result<Self, String> {
        match fs::read_to_string(repo.join(MANIFEST)) {
            Ok(text) => serde_json::from_str(&text).map_err(|err| format!("{MANIFEST}: {err}")),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(format!("{MANIFEST}: {err}")),
        }
    }

    /// The job's pick: the one recorded, else the job's default.
    pub(crate) fn pick(&self, job: &str) -> &str {
        match self.picks.get(job) {
            Some(pick) => pick,
            None => JOBS
                .iter()
                .find(|(name, _)| *name == job)
                .map_or(NONE, |(_, suggestions)| suggestions[0].0),
        }
    }

    /// A skill recorded here that `worktree`, a checkout of a Ticket's base,
    /// lacks: added in /config and not yet merged.
    pub(crate) fn unmerged(&self, worktree: &Path, name: &str) -> bool {
        self.skills.contains_key(name) && !skill_dir(worktree, name).exists()
    }

    /// A Stage skill with each job's placeholder filled in with the job's
    /// pick, after the App's mention prefix. A pick of none drops the
    /// placeholder's line, leaving the Stage skill's own instruction around it, and so
    /// does one lacking from have (list's names) and built_in; those are
    /// given back too, each as "pick (job)".
    pub(crate) fn fill_jobs(
        &self,
        skill: &str,
        have: &[String],
        built_in: &[&str],
        mention: &str,
    ) -> (String, Vec<String>) {
        let mut lacking = Vec::new();
        let lines: Vec<String> = skill
            .lines()
            .filter_map(|line| {
                let mut line = line.to_string();
                for (job, _) in JOBS {
                    let held = placeholder(job);
                    if !line.contains(&held) {
                        continue;
                    }
                    let pick = self.pick(job);
                    if lacks(pick, have, built_in) {
                        let said = format!("{pick} ({job})");
                        if !lacking.contains(&said) {
                            lacking.push(said);
                        }
                        return None;
                    }
                    if pick == NONE {
                        return None;
                    }
                    line = line.replace(&held, &format!("{mention}{pick}"));
                }
                Some(line)
            })
            .collect();
        (lines.join("\n"), lacking)
    }

    /// Written to a temp file and renamed into place, so that a cut-short
    /// write never leaves a manifest load() refuses.
    pub(crate) fn save(&self, repo: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).unwrap() + "\n";
        let path = repo.join(MANIFEST);
        let tmp = path.with_extension("json.tmp");
        fs::create_dir_all(repo.join(".orqadence"))
            .and_then(|()| fs::write(&tmp, text))
            .and_then(|()| fs::rename(&tmp, &path))
            .map_err(|err| format!("{MANIFEST}: {err}"))
    }
}

/// Where a skill comes from: a clone URL, the ref to clone (empty for the
/// default branch) and the skill's folder in the clone (empty to search it).
#[derive(Debug, PartialEq)]
pub(crate) struct Source {
    pub(crate) repo: String,
    pub(crate) git_ref: String,
    pub(crate) path: String,
}

/// Reads a source as the user gives it: owner/repo, owner/repo/path, a
/// GitHub URL (with /tree/<ref>/<path> or not), or any other git URL. A bare
/// name is refused: it needs a source, and skills.sh finds one.
pub(crate) fn parse_source(text: &str) -> Result<Source, String> {
    let text = text.trim();
    if !text.contains('/') && !text.contains(':') {
        return Err(format!("'{text}' is a skill name, not a source: find its source on skills.sh (https://skills.sh/?q={text}), then add it as owner/repo or its URL"));
    }
    let github = ["https://github.com/", "http://github.com/", "github.com/"]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix));
    let Some(rest) = github.or_else(|| (!text.contains(':')).then_some(text)) else {
        // Any other git URL, cloned as it is.
        return Ok(Source {
            repo: text.to_string(),
            git_ref: String::new(),
            path: String::new(),
        });
    };
    let parts: Vec<&str> = rest.split('/').filter(|part| !part.is_empty()).collect();
    let [owner, name, path @ ..] = parts.as_slice() else {
        return Err(format!(
            "'{text}' is not a source: give owner/repo or a git URL"
        ));
    };
    // ponytail: the ref is one segment, so a branch named with a slash reads
    // as ref plus path; take owner/repo@... if one is ever needed.
    let (git_ref, path) = match path {
        ["tree", git_ref, path @ ..] if github.is_some() => (*git_ref, path),
        _ => ("", path),
    };
    if path.contains(&"..") {
        return Err(format!("'{text}': a path cannot leave its repo"));
    }
    Ok(Source {
        repo: format!(
            "https://github.com/{owner}/{}",
            name.trim_end_matches(".git")
        ),
        git_ref: git_ref.to_string(),
        path: path.join("/"),
    })
}

/// What add did.
#[derive(Debug, PartialEq)]
pub(crate) enum Added {
    Installed(String),
    /// The source holds several skills and none was named: their names, for
    /// the caller to pick from.
    Choose(Vec<String>),
}

/// Installs one skill from a source: the one named, with PREFIX or
/// without, or the only one there. The skill is copied to its folder in
/// .orqadence/skills under its name with PREFIX, which its SKILL.md takes
/// too, linked as init does, and recorded with its source and commit;
/// Installed gives that name. A source already installed is refused, and so
/// is a same-named skill from anywhere else.
pub(crate) fn add(
    repo: &Path,
    tools: &dyn Tools,
    source: &str,
    name: Option<&str>,
) -> Result<Added, String> {
    let name = name.map(|name| name.strip_prefix(PREFIX).unwrap_or(name));
    let source = parse_source(source)?;
    let mut manifest = Manifest::load(repo)?;
    let (tmp, commit) = fetch(repo, tools, &source.repo, &source.git_ref)?;
    // Canonical, so that find gives each skill's folder from the clone's root.
    let clone = tmp.path().canonicalize().map_err(|err| err.to_string())?;
    let dir = in_clone(&clone, &source.path)
        .ok_or_else(|| format!("no folder {} inside {}", source.path, source.repo))?;
    let mut skills = Vec::new();
    find(&clone, &dir, &mut skills);
    skills.sort();
    let (name, path) = match (name, skills.len()) {
        (Some(name), _) => match skills.iter().position(|(found, _)| found == name) {
            Some(i) => skills.swap_remove(i),
            None => {
                let names: Vec<_> = skills.iter().map(|(found, _)| found.as_str()).collect();
                return Err(format!(
                    "no skill named {name} in {}: it has {}",
                    source.repo,
                    names.join(", ")
                ));
            }
        },
        (None, 0) => return Err(format!("no skill in {}", source.repo)),
        (None, 1) => skills.remove(0),
        (None, _) => {
            return Ok(Added::Choose(
                skills.into_iter().map(|(found, _)| found).collect(),
            ))
        }
    };
    let name = format!("{PREFIX}{name}");
    if let Some((other, _)) = manifest
        .skills
        .iter()
        .find(|(_, skill)| skill.repo == source.repo && skill.path == path)
    {
        return Err(format!(
            "{}/{path} is already installed, as {other}: update it instead",
            source.repo
        ));
    }
    match manifest.skills.get(&name) {
        Some(skill) if skill.shipped => {
            return Err(format!("{name} is a Shipped skill: it cannot be replaced"))
        }
        Some(skill) => {
            return Err(format!(
                "a skill named {name} from {} is installed: remove it first",
                skill.repo
            ))
        }
        None => {}
    }
    // put and its undo write through all three folders.
    if let Some(dir) = unowned(repo) {
        return Err(format!(
            "{dir} is not the checkout's own folder: Orqadence will not install {name} there"
        ));
    }
    let (at, links) = (skill_dir(repo, &name), links(repo, &name));
    if let Some(there) = iter::once(&at)
        .chain(&links)
        .find(|path| fs::symlink_metadata(path).is_ok())
    {
        return Err(format!(
            "{} is there already, and Orqadence did not put it there",
            there.display()
        ));
    }
    let installed = put(repo, &clone.join(&path), &name)
        .map_err(|err| format!("{}: {err}", at.display()))
        .and_then(|()| {
            let skill = Installed {
                repo: source.repo.clone(),
                git_ref: source.git_ref.clone(),
                path,
                commit,
                shipped: false,
            };
            manifest.skills.insert(name.clone(), skill);
            manifest.save(repo)
        });
    if installed.is_err() {
        // Nothing left half installed, which a later add would take for the
        // repo's own.
        for link in &links {
            let _ = fs::remove_file(link);
        }
        let _ = fs::remove_dir_all(&at);
    }
    installed.map(|()| Added::Installed(name))
}

/// Fetches an installed skill's source again, replaces its folder with the
/// skill at the recorded path, named as installed, and records the new
/// commit.
pub(crate) fn update(repo: &Path, tools: &dyn Tools, name: &str) -> Result<(), String> {
    let mut manifest = Manifest::load(repo)?;
    let skill = third_party(repo, &manifest, name)?.clone();
    let (clone, commit) = fetch(repo, tools, &skill.repo, &skill.git_ref)?;
    let upstream = name.strip_prefix(PREFIX).unwrap_or(name);
    let from = in_clone(clone.path(), &skill.path)
        .filter(|from| skill_in(from).as_deref() == Some(upstream))
        .ok_or_else(|| format!("{} no longer has {name} at {}", skill.repo, skill.path))?;
    // The new copy goes beside the old one first, and the old one is only
    // moved aside until the new one is in place, so that a copy or a rename
    // that fails leaves the installed skill whole.
    let at = skill_dir(repo, name);
    let fresh = at.with_file_name(format!(".{name}.new"));
    let old = at.with_file_name(format!(".{name}.old"));
    // A staging folder an earlier update left that cannot be cleared would
    // carry its stale files into the new copy.
    match fs::remove_dir_all(&fresh) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            return Err(format!("{}: {err}", fresh.display()))
        }
        _ => {}
    }
    let _ = fs::remove_dir_all(&old);
    let swapped = copy_dir(&from, &fresh, false).and_then(|()| {
        rename(&fresh, name)?;
        if at.exists() {
            fs::rename(&at, &old)?;
        }
        fs::rename(&fresh, &at).inspect_err(|_| {
            let _ = fs::rename(&old, &at);
        })
    });
    if let Err(err) = swapped {
        let _ = fs::remove_dir_all(&fresh);
        return Err(format!("{}: {err}", at.display()));
    }
    // The old copy goes only once the manifest records the new commit: a
    // failed save puts it back.
    manifest.skills.get_mut(name).unwrap().commit = commit;
    let saved = manifest.save(repo);
    if saved.is_ok() {
        let _ = fs::remove_dir_all(&old);
    } else {
        let _ = fs::remove_dir_all(&at);
        let _ = fs::rename(&old, &at);
    }
    saved
}

/// Updates every installed skill but the Shipped ones, and returns those
/// that failed, each with why.
pub(crate) fn update_all(repo: &Path, tools: &dyn Tools) -> Result<Vec<(String, String)>, String> {
    Ok(Manifest::load(repo)?
        .skills
        .into_iter()
        .filter(|(_, skill)| !skill.shipped)
        .filter_map(|(name, _)| update(repo, tools, &name).err().map(|err| (name, err)))
        .collect())
}

/// Removes a skill Orqadence fetched: its folder, both links and its entry.
/// A job it did, picked or by default, is set to none.
pub(crate) fn remove(repo: &Path, name: &str) -> Result<(), String> {
    let mut manifest = Manifest::load(repo)?;
    third_party(repo, &manifest, name)?;
    // Only the links put makes are removed, not one the user put there
    // instead, and a failed save puts them back.
    let links = links(repo, name);
    if let Some(dir) = unowned(repo).filter(|_| links.iter().any(|l| fs::read_link(l).is_ok())) {
        return Err(format!(
            "{dir} is not the checkout's own folder: Orqadence will not touch {name}"
        ));
    }
    let target = target(name);
    let links: Vec<PathBuf> = links
        .into_iter()
        .filter(|link| fs::read_link(link).is_ok_and(|to| to == target))
        .collect();
    for link in &links {
        fs::remove_file(link).map_err(|err| format!("{}: {err}", link.display()))?;
    }
    for (job, _) in JOBS {
        if manifest.pick(job) == name {
            manifest.picks.insert(job.to_string(), NONE.to_string());
        }
    }
    manifest.skills.remove(name);
    // The folder is moved aside until the manifest is saved without it, and
    // back should the save fail.
    let at = skill_dir(repo, name);
    let old = at.with_file_name(format!(".{name}.old"));
    let _ = fs::remove_dir_all(&old);
    let removed = if at.exists() {
        fs::rename(&at, &old)
    } else {
        Ok(())
    }
    .map_err(|err| format!("{}: {err}", at.display()))
    .and_then(|()| {
        manifest.save(repo).inspect_err(|_| {
            let _ = fs::rename(&old, &at);
        })
    });
    if removed.is_ok() {
        let _ = fs::remove_dir_all(&old);
    } else {
        for link in links {
            let _ = symlink(&target, link);
        }
    }
    removed
}

/// An installed skill Orqadence fetched from a source, which update and
/// remove may touch. Only while its name is a folder name and
/// .orqadence/skills is the checkout's own: they delete
/// .orqadence/skills/<name>, and an entry edited by hand, or a linked
/// folder, could name something else.
fn third_party<'a>(
    repo: &Path,
    manifest: &'a Manifest,
    name: &str,
) -> Result<&'a Installed, String> {
    match manifest.skills.get(name) {
        None => Err(format!("{name} is not installed by Orqadence")),
        Some(skill) if skill.shipped => Err(format!(
            "{name} is a Shipped skill: it cannot be removed, and orqa init updates it"
        )),
        Some(_) if !safe_name(name) => Err(format!(
            "{name} in {MANIFEST} is not a folder name: Orqadence will not touch it"
        )),
        Some(_) if !own(repo, FILES) => Err(format!(
            "{FILES} is not the checkout's own folder: Orqadence will not touch {name}"
        )),
        Some(skill) => Ok(skill),
    }
}

/// Whether dir, from the checkout's root, is the checkout's own: no link on
/// the way leads anywhere else, not even elsewhere in the checkout. A folder
/// not there yet is its own: add makes it there.
fn own(repo: &Path, dir: &str) -> bool {
    Path::new(dir)
        .ancestors()
        .filter(|part| !part.as_os_str().is_empty())
        .all(|part| match fs::symlink_metadata(repo.join(part)) {
            Ok(meta) => !meta.file_type().is_symlink(),
            Err(err) => err.kind() == io::ErrorKind::NotFound,
        })
}

/// The folder at path in the clone, resolved, when it is inside the clone: a
/// link there could lead anywhere on this machine.
fn in_clone(clone: &Path, path: &str) -> Option<PathBuf> {
    let root = clone.canonicalize().ok()?;
    let dir = clone.join(path).canonicalize().ok()?;
    (dir.starts_with(&root) && dir.is_dir()).then_some(dir)
}

/// Shallow-clones url at git_ref (the default branch when empty) into a
/// scratch directory, removed on Drop, and gives it with its commit. No
/// terminal prompt: a mistyped GitHub repo asks for a password, which would
/// hang the Shell.
fn fetch(
    repo: &Path,
    tools: &dyn Tools,
    url: &str,
    git_ref: &str,
) -> Result<(TempDir, String), String> {
    let clone = TempDir::create().map_err(|err| err.to_string())?;
    let dest = clone.path().to_string_lossy().into_owned();
    let mut argv = vec![
        "env",
        "GIT_TERMINAL_PROMPT=0",
        "git",
        "clone",
        "--depth",
        "1",
        "--quiet",
    ];
    if !git_ref.is_empty() {
        argv.extend(["--branch", git_ref]);
    }
    argv.extend(["--", url, &dest]);
    let commit = tools
        .run(repo, &argv)
        .and_then(|_| tools.run(clone.path(), &["git", "rev-parse", "HEAD"]))
        .map_err(|err| err.to_string())?;
    Ok((clone, commit.trim().to_string()))
}

/// Every skill under dir, as (name, folder from the clone's root).
fn find(clone: &Path, dir: &Path, into: &mut Vec<(String, String)>) {
    if let Some(name) = skill_in(dir) {
        let path = dir.strip_prefix(clone).unwrap_or(dir);
        into.push((name, path.to_string_lossy().into_owned()));
    }
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) && entry.file_name() != ".git" {
            find(clone, &entry.path(), into);
        }
    }
}

/// The name of the skill in dir. Its SKILL.md must be a file, not a link:
/// copy_dir copies no links from a clone, so the skill would be installed
/// without it.
fn skill_in(dir: &Path) -> Option<String> {
    let skill = dir.join("SKILL.md");
    fs::symlink_metadata(&skill)
        .ok()
        .filter(|meta| meta.is_file())?;
    skill_name(&fs::read_to_string(skill).ok()?)
}

/// The name in a SKILL.md's frontmatter, when it is safe.
fn skill_name(skill: &str) -> Option<String> {
    let mut lines = skill.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let name = lines
        .take_while(|line| line.trim() != "---")
        .find_map(|line| line.strip_prefix("name:"))?
        .trim()
        .trim_matches(['"', '\'']);
    safe_name(name).then(|| name.to_string())
}

/// A name safe as a folder name (it comes from someone else's repo) that
/// cannot be read as the pick none.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name != NONE
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Copies the skill's folder into .orqadence/skills, named name, and links
/// it there.
fn put(repo: &Path, from: &Path, name: &str) -> io::Result<()> {
    copy_dir(from, &skill_dir(repo, name), false)?;
    rename(&skill_dir(repo, name), name)?;
    link(repo, name)
}

/// Gives the skill in dir the name name in its SKILL.md, as the agents
/// name a skill by it. A SKILL.md that is a link is left: it leads out of
/// .orqadence/skills.
fn rename(dir: &Path, name: &str) -> io::Result<()> {
    let path = dir.join("SKILL.md");
    if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
        return Ok(());
    }
    fs::write(&path, renamed(&fs::read_to_string(&path)?, name))
}

/// A SKILL.md's text with name on its frontmatter's name line.
pub(crate) fn renamed(skill: &str, name: &str) -> String {
    let mut fences = 0;
    skill
        .split_inclusive('\n')
        .map(|line| {
            if line.trim() == "---" {
                fences += 1;
            }
            match fences == 1 && line.starts_with("name:") {
                true => format!("name: {name}\n"),
                false => line.to_string(),
            }
        })
        .collect()
}

/// Each (old name, new name).
type Renames = Vec<(String, String)>;

/// Renames each skill the manifest names without PREFIX, as an init from
/// before it left them, to its name with it: its folder in
/// .orqadence/skills and the name in its SKILL.md, its links, its entry and
/// each pick of it. The shipped create-pr an init put beside the repo's own
/// as orqadence-create-pr becomes CREATE_PR. One the manifest names already
/// with it (a merge, or create-pr and orqadence-create-pr both) is stale:
/// its folder, links and entry go, its picks turn to the new one. Gives
/// each (old, new) renamed, then each left for the user: a stale one in a
/// .orqadence/skills not the checkout's own (own).
pub(crate) fn prefix(repo: &Path, manifest: &mut Manifest) -> io::Result<(Renames, Renames)> {
    let old: Vec<String> = manifest
        .skills
        .keys()
        .filter(|name| !name.starts_with(PREFIX) && safe_name(name))
        .cloned()
        .collect();
    let (mut renamed, mut left) = (Vec::new(), Vec::new());
    for name in old {
        let new = match name.as_str() {
            "orqadence-create-pr" => CREATE_PR.to_string(),
            _ => format!("{PREFIX}{name}"),
        };
        let (from, to) = (skill_dir(repo, &name), skill_dir(repo, &new));
        let stale = manifest.skills.contains_key(&new);
        let had = fs::symlink_metadata(&from).is_ok();
        if stale && had && !own(repo, FILES) {
            left.push((name, new));
            continue;
        }
        if own(repo, FILES) && had {
            match (fs::symlink_metadata(&to).is_ok(), stale) {
                (false, _) => {
                    fs::rename(&from, &to)?;
                    rename(&to, &new)?;
                }
                (true, true) => fs::remove_dir_all(&from)?,
                (true, false) => {}
            }
        }
        for link in links(repo, &name) {
            if fs::read_link(&link).is_ok_and(|to| to == target(&name)) {
                fs::remove_file(link)?;
            }
        }
        if to.exists() {
            link(repo, &new)?;
        }
        let entry = manifest.skills.remove(&name).unwrap();
        manifest.skills.entry(new.clone()).or_insert(entry);
        for pick in manifest.picks.values_mut().filter(|pick| **pick == name) {
            *pick = new.clone();
        }
        renamed.push((name, new));
    }
    Ok((renamed, left))
}

/// Whether the user has turned on their personal skills (PERSONAL) for
/// this checkout. A file missing or unreadable is off.
pub(crate) fn personal(repo: &Path) -> bool {
    fs::read_to_string(repo.join(on_call::CONFIG))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|doc| doc[PERSONAL] == true)
}

/// Turns the user's personal skills on or off for this checkout.
pub(crate) fn set_personal(repo: &Path, on: bool) -> io::Result<()> {
    on_call::keep(repo, PERSONAL, serde_json::Value::Bool(on))
}

/// Links the skill in .orqadence/skills from .agents/skills and
/// .claude/skills, relative, leaving whatever is at either link's path
/// already: Orqadence's link, or the repo's own. A links folder that is not
/// the checkout's own (own) gets none: it could be ~/.claude, where the link
/// would dangle.
pub(crate) fn link(repo: &Path, name: &str) -> io::Result<()> {
    for (dir, link) in LINKS.iter().zip(links(repo, name)) {
        if own(repo, dir) && fs::symlink_metadata(&link).is_err() {
            fs::create_dir_all(link.parent().unwrap())?;
            symlink(target(name), link)?;
        }
    }
    Ok(())
}

/// Removes from a Ticket's worktree the links an Orqadence from before ADR
/// 0006 made there, absolute ones to the checkout's .orqadence/skills: left,
/// a resumed Ticket would run the checkout's skills, not its base's. The
/// worktree's own links, relative, stay. One that cannot be removed, or a
/// links folder that cannot be read, is the error: the Ticket must not start
/// with it.
pub(crate) fn unlink_checkout_skills(repo: &Path, worktree: &Path) -> io::Result<()> {
    let from = repo.join(FILES);
    for dir in LINKS.iter().filter(|dir| own(worktree, dir)) {
        let entries = match fs::read_dir(worktree.join(dir)) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            entries => entries?,
        };
        for entry in entries {
            let entry = entry?;
            if fs::read_link(entry.path()).is_ok_and(|to| to.is_absolute() && to.starts_with(&from))
            {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}

/// Puts each skill the manifest names into .orqadence/skills from where an
/// init from before ADR 0006 put it, and links it: from the repo's
/// .agents/skills it moves (move_in); from user level, ~/.agents/skills, it
/// is copied as it is, edits and links kept, and the ~ copy stays for other
/// checkouts. The checkout's are there already.
pub(crate) fn settle(repo: &Path, home: &Path, manifest: &Manifest) -> io::Result<()> {
    // The location field that init wrote then, gone from Manifest.
    let old: serde_json::Value = fs::read_to_string(repo.join(MANIFEST))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let user = old["location"] == "user";
    if user && home.as_os_str().is_empty() {
        return Err(io::Error::other(
            "no HOME, so the skills at user level cannot be copied into .orqadence/skills",
        ));
    }
    for name in manifest.skills.keys().filter(|name| safe_name(name)) {
        let at = skill_dir(repo, name);
        if fs::symlink_metadata(&at).is_err() {
            let mine = home.join(".agents/skills").join(name);
            match user {
                false => move_in(repo, name)?,
                true if mine.is_dir() => copy_dir(&mine, &at, true)?,
                true => {}
            }
        }
        if at.exists() {
            link(repo, name)?;
        }
    }
    Ok(())
}

/// Moves a skill folder of the repo's own .agents/skills, as an older init
/// left it there, into .orqadence/skills, and drops the link that init made
/// to it from .claude/skills; link makes the new ones. Nothing when
/// .orqadence/skills has it, or when .agents/skills is a link, which could
/// lead to another checkout's.
pub(crate) fn move_in(repo: &Path, name: &str) -> io::Result<()> {
    let (old, at) = (
        repo.join(".agents/skills").join(name),
        skill_dir(repo, name),
    );
    if fs::symlink_metadata(&at).is_ok()
        || !own(repo, ".agents/skills")
        || !fs::symlink_metadata(&old).is_ok_and(|meta| meta.is_dir())
    {
        return Ok(());
    }
    fs::create_dir_all(at.parent().unwrap())?;
    fs::rename(&old, &at)?;
    let claude = repo.join(".claude/skills").join(name);
    if fs::read_link(&claude).is_ok_and(|to| to == Path::new("../../.agents/skills").join(name)) {
        fs::remove_file(claude)?;
    }
    Ok(())
}

/// Copies files and folders, and links as links when links is set. A clone
/// is copied without them, so that a link in it cannot pull in anything from
/// outside it; a skill being moved keeps them, as a rename would.
pub(crate) fn copy_dir(from: &Path, to: &Path, links: bool) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if kind.is_dir() && entry.file_name() != ".git" {
            copy_dir(&entry.path(), &dest, links)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), dest)?;
        } else if links && kind.is_symlink() {
            // Over the same link, when copied back over the skill it came from.
            let _ = fs::remove_file(&dest);
            symlink(fs::read_link(entry.path())?, dest)?;
        }
    }
    Ok(())
}

/// A Claude Code plugin, as claude plugin list --json gives it.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Plugin {
    id: String,
    enabled: bool,
    install_path: PathBuf,
}

/// The enabled Claude Code plugins, each by its name (its id before the @)
/// with its install path. Without claude, none.
pub(crate) fn plugins(repo: &Path, tools: &dyn Tools) -> Vec<(String, PathBuf)> {
    let plugins: Vec<Plugin> = tools
        .run(repo, &["claude", "plugin", "list", "--json"])
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    plugins
        .into_iter()
        .filter(|plugin| plugin.enabled)
        .map(|plugin| {
            let name = plugin.id.split('@').next().unwrap_or_default();
            (name.to_string(), plugin.install_path)
        })
        .collect()
}

/// Every skill the user has, each with where it is: the repo's, Orqadence's
/// in .orqadence/skills and, when personal (the user's switch), the user's
/// and the enabled Claude Code plugins', named plugin:skill. A skill is
/// named by its folder, as the agents name it. A skill linked from
/// .claude/skills to .agents/skills is listed at both places: Claude reads
/// the one, codex the other. No home, no user's.
// ponytail: a plugin's skills are read from its skills/ folder only, not a
// skills list in its plugin.json; read that when a plugin needs it.
pub(crate) fn list(
    repo: &Path,
    home: &Path,
    tools: &dyn Tools,
    personal: bool,
) -> Vec<(String, PathBuf)> {
    let mut dirs = vec![
        repo.join(".agents/skills"),
        repo.join(".claude/skills"),
        repo.join(FILES),
    ];
    if personal && !home.as_os_str().is_empty() {
        dirs.extend([home.join(".claude/skills"), home.join(".agents/skills")]);
    }
    let dirs = dirs.into_iter().map(|dir| (String::new(), dir));
    let plugins = match personal {
        true => plugins(repo, tools),
        false => Vec::new(),
    };
    let plugin_dirs = plugins
        .into_iter()
        .map(|(name, path)| (format!("{name}:"), path.join("skills")));
    let mut found = Vec::new();
    for (prefix, dir) in dirs.into_iter().chain(plugin_dirs) {
        let mut skills: Vec<PathBuf> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("SKILL.md").is_file())
            .collect();
        skills.sort();
        for path in skills {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            found.push((format!("{prefix}{name}"), path));
        }
    }
    found
}
