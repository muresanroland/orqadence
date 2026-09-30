//! Silent self-update from GitHub Releases (harness-7bj.10): the check reads
//! the tag from the /releases/latest redirect, a strictly greater version is
//! downloaded to `<exe>.new.<pid>` beside the exe, and the rename over the exe
//! is the caller's act, taken only under the repo lock. Each check also
//! keeps the prices of the models in use from LiteLLM's catalog in
//! .orqadence-local/prices.json, for the Epic summary. A dev build never
//! checks. The check and the downloads sit behind `Releases`, with the ureq
//! `GitHub` behind it and a fake in tests. A done rename leaves
//! `<exe>.updated` holding the tag, which the next Shell takes for its
//! update notice.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::orchestrator::app::{self, ROWS};
use crate::orchestrator::cost::{keep_prices, prices_file};
use crate::orchestrator::state::acquire_lock;
use crate::tools::Tools;

const REPO: &str = "muresanroland/orqadence";
/// LiteLLM's price catalog: every model's list prices per token, by the name
/// its provider's API gives it.
const CATALOG: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
/// The release asset is orqa-<target>, the target fixed at compile time,
/// with the magic its binary starts with; any other target never checks.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(crate) const TARGET: Option<(&str, [u8; 4])> =
    Some(("aarch64-apple-darwin", [0xcf, 0xfa, 0xed, 0xfe])); // Mach-O 64
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) const TARGET: Option<(&str, [u8; 4])> =
    Some(("x86_64-unknown-linux-gnu", [0x7f, b'E', b'L', b'F']));
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
pub(crate) const TARGET: Option<(&str, [u8; 4])> = None;

/// The Shell checks again this often while it stays open.
pub(crate) const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const TIMEOUT: Duration = Duration::from_secs(5);
/// A release binary is a few MB, and so is the price catalog; the body gets
/// longer than the handshakes.
const BODY_TIMEOUT: Duration = Duration::from_secs(60);

/// The releases of Orqadence: the latest tag, and one asset of one release.
pub(crate) trait Releases: Send + Sync {
    fn latest_tag(&self) -> Result<String, String>;
    /// Writes the release's orqa-<target> asset to `dest`, whole or not
    /// at all: a body shorter than its Content-Length is an error.
    fn download(&self, tag: &str, target: &str, dest: &Path) -> Result<(), String>;
    /// LiteLLM's price catalog.
    fn catalog(&self) -> Result<Value, String>;
}

/// A downloaded release beside the exe, waiting for the rename.
#[derive(Debug)]
pub(crate) struct Ready {
    pub(crate) tag: String,
    tmp: PathBuf,
    exe: PathBuf,
}

impl Ready {
    /// The rename over the exe: atomic, and the running image keeps its old
    /// inode. The temp file goes on a refusal. A done rename leaves the
    /// marker; a marker that cannot be written loses only the notice.
    pub(crate) fn install(self) -> Result<(), String> {
        fs::rename(&self.tmp, &self.exe).map_err(|err| {
            self.discard();
            format!("rename refused: {err}")
        })?;
        let _ = fs::write(marker(&self.exe), &self.tag);
        Ok(())
    }

    pub(crate) fn discard(&self) {
        let _ = fs::remove_file(&self.tmp);
    }
}

/// One check's outcome, as the updater thread hands it over.
pub(crate) type Checked = Result<Option<Ready>, String>;

/// One check for `version`, the exe at `exe`: Ok(None) is up to date, Ok(Some)
/// a greater release downloaded and chmod 755 beside the exe, Err one reason
/// with no temp file left. A dev build (v1.0.0-dev), any version that is not
/// vX.Y.Z and an unsupported target never ask.
pub(crate) fn check(releases: &dyn Releases, version: &str, exe: &Path) -> Checked {
    let (Some((target, magic)), Some(current)) = (TARGET, semver(version)) else {
        return Ok(None);
    };
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("no exe name in {}", exe.display()))?;
    sweep(exe, name);
    let tag = releases.latest_tag()?;
    let latest = semver(&tag).ok_or_else(|| format!("unreadable tag {tag:?}"))?;
    if latest <= current {
        return Ok(None);
    }
    let ready = Ready {
        tag,
        tmp: exe.with_file_name(format!("{name}.new.{}", std::process::id())),
        exe: exe.to_path_buf(),
    };
    let fetched = releases
        .download(&ready.tag, target, &ready.tmp)
        .and_then(|()| {
            let mut head = [0; 4];
            let read = File::open(&ready.tmp).and_then(|mut f| f.read_exact(&mut head));
            match read {
                Ok(()) if head == magic => Ok(()),
                _ => Err(format!("not a {target} binary")),
            }
        })
        .and_then(|()| {
            fs::set_permissions(&ready.tmp, fs::Permissions::from_mode(0o755))
                .map_err(|err| err.to_string())
        });
    if let Err(err) = fetched {
        ready.discard();
        return Err(err);
    }
    Ok(Some(ready))
}

/// Removes the `<name>.new.<pid>` files beside the exe that another process
/// left when it died mid-download.
// ponytail: a live Shell's pending download goes too; its rename then fails
// once, and this process installs the same release.
fn sweep(exe: &Path, name: &str) {
    let Some(dir) = exe.parent() else {
        return;
    };
    let (prefix, ours) = (format!("{name}.new."), std::process::id().to_string());
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name();
        let pid = file.to_str().and_then(|f| f.strip_prefix(&prefix));
        if pid.is_some_and(|p| p != ours && !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// `<exe>.updated`, holding the tag the last install put in place.
fn marker(exe: &Path) -> PathBuf {
    let mut path = exe.as_os_str().to_owned();
    path.push(".updated");
    path.into()
}

/// Whether an install put `version` in place, once: the marker beside `exe`
/// is deleted whatever it names. One naming another version (an install
/// undone since, or a dev build) is false, as is a missing or unreadable one.
pub(crate) fn take_marker(exe: &Path, version: &str) -> bool {
    let marker = marker(exe);
    let Ok(tag) = fs::read(&marker) else {
        return false;
    };
    let _ = fs::remove_file(marker);
    tag == version.as_bytes()
}

/// The release's page on github.com.
pub(crate) fn release_notes(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/{tag}")
}

/// The Shell's check of the prices: the catalog's for the models the rows
/// name now, kept over prices.json's, so a model a row changed to is added.
/// A dev build never asks.
pub(crate) fn refresh_prices(
    releases: &dyn Releases,
    version: &str,
    repo: &Path,
) -> Result<(), String> {
    if semver(version).is_none() {
        return Ok(());
    }
    let catalog = releases.catalog()?;
    keep_prices(repo, &catalog, &in_use(repo)).map_err(|err| err.to_string())?;
    Ok(())
}

/// init's prices.json, made once: the prices of the models the rows name,
/// and of every model their Apps on PATH list, claude its aliases and codex
/// its catalog, so a row's default is priced too. A failure is one line,
/// and the next init tries again; a dev build never asks.
pub(crate) fn seed_prices(
    repo: &Path,
    tools: &dyn Tools,
    releases: &dyn Releases,
    version: &str,
    out: &mut dyn Write,
) {
    if semver(version).is_none() || prices_file(repo).exists() {
        return;
    }
    let doc = app::read(repo).map(|(_, doc)| doc).unwrap_or_default();
    let apps: BTreeSet<String> = ROWS
        .iter()
        .filter_map(|key| app::field(&doc, key, "app").ok())
        .collect();
    let listed = apps
        .iter()
        .filter_map(|name| app::app(name))
        .filter(|app| tools.run(repo, &["which", app.bin]).is_ok())
        .flat_map(|app| (app.models)(tools, repo).unwrap_or_default())
        .filter_map(|(model, _)| priced_as(&model));
    let models = in_use(repo).into_iter().chain(listed).collect();
    let kept = releases
        .catalog()
        .and_then(|catalog| keep_prices(repo, &catalog, &models).map_err(|err| err.to_string()));
    let _ = match kept {
        Ok(kept) => writeln!(
            out,
            "init: prices of {} models kept in {}",
            kept.len(),
            prices_file(repo).strip_prefix(repo).unwrap().display()
        ),
        Err(err) => writeln!(
            out,
            "init: prices not fetched, so the Epic summary shows its costs unpriced; run orqa init again to retry: {err}"
        ),
    };
}

/// The models config.json's rows name, Implement's plan model too, by the
/// names the catalog lists them under.
fn in_use(repo: &Path) -> BTreeSet<String> {
    let doc = app::read(repo).map(|(_, doc)| doc).unwrap_or_default();
    ROWS.iter()
        .flat_map(|key| ["model", "plan_model"].map(|name| app::field(&doc, key, name)))
        .filter_map(|model| priced_as(&model.ok()?))
        .collect()
}

/// A row's model as the catalog names it: a claude alias as its full id,
/// with no [1m] after it; default and none name no model.
fn priced_as(model: &str) -> Option<String> {
    let model = model.split('[').next().unwrap_or_default();
    (!matches!(model, "" | "default" | "none")).then(|| app::full_id(model))
}

/// v1.2.3 as (1, 2, 3); anything else is None.
fn semver(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v')?.splitn(3, '.');
    let mut next = || parts.next()?.parse().ok();
    Some((next()?, next()?, next()?))
}

/// The running binary's real path: a symlink is followed, so the file behind
/// it is replaced, not the link.
pub(crate) fn exe_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|err| format!("no exe path: {err}"))
}

/// Replaces this process with `exe` and the same argv, argv[0] included;
/// returns only on failure.
pub(crate) fn reexec(exe: &Path) -> String {
    let mut args = std::env::args_os();
    let arg0 = args.next().unwrap_or_else(|| exe.into());
    let err = Command::new(exe).arg0(arg0).args(args).exec();
    format!("re-exec refused: {err}")
}

/// init's check, before the gate: a greater release installs under the repo
/// lock and the process re-execs, so init installs skills from the binary it
/// just fetched. A run holding the lock skips it silently, the temp file gone
/// and the next check trying again; any other failure is one line, and init
/// goes on.
pub(crate) fn at_init(repo: &Path, out: &mut dyn Write) {
    let version = crate::version::version();
    let checked = exe_path().and_then(|exe| Ok((check(&GitHub, &version, &exe)?, exe)));
    let err = match checked {
        Ok((Some(ready), exe)) => {
            let tag = ready.tag.clone();
            let installed = match acquire_lock(repo) {
                Ok(_lock) => ready.install(),
                Err(_) => return ready.discard(),
            };
            match installed {
                Ok(()) => {
                    let _ = writeln!(out, "updating to {tag}");
                    reexec(&exe)
                }
                Err(err) => err,
            }
        }
        Ok((None, _)) => return,
        Err(err) => err,
    };
    let _ = writeln!(out, "update check failed: {err}");
}

/// The releases on github.com, over ureq.
pub(crate) struct GitHub;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(TIMEOUT))
        .timeout_send_request(Some(TIMEOUT))
        .timeout_recv_response(Some(TIMEOUT))
        .timeout_recv_body(Some(BODY_TIMEOUT))
        .build()
        .into()
}

impl Releases for GitHub {
    /// The tag is the last segment of the /releases/latest redirect's
    /// Location, read without following it; no API call, no rate limit.
    fn latest_tag(&self) -> Result<String, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(TIMEOUT))
            .max_redirects(0)
            .build()
            .into();
        let resp = agent
            .head(&format!("https://github.com/{REPO}/releases/latest"))
            .call()
            .map_err(|err| err.to_string())?;
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| format!("no redirect from /releases/latest ({})", resp.status()))?;
        match location.rsplit_once("/releases/tag/") {
            Some((_, tag)) if !tag.is_empty() => Ok(tag.to_string()),
            _ => Err(format!("no tag in redirect to {location}")),
        }
    }

    fn download(&self, tag: &str, target: &str, dest: &Path) -> Result<(), String> {
        let url = format!("https://github.com/{REPO}/releases/download/{tag}/orqa-{target}");
        let mut resp = agent().get(&url).call().map_err(|err| err.to_string())?;
        let want = resp.body().content_length();
        let mut file = File::create(dest).map_err(|err| err.to_string())?;
        let got = io::copy(&mut resp.body_mut().as_reader(), &mut file)
            .and_then(|n| file.sync_all().map(|()| n))
            .map_err(|err| err.to_string())?;
        match want {
            Some(want) if want != got => Err(format!("truncated body: {got} of {want} bytes")),
            None => Err("no Content-Length on the asset".to_string()),
            Some(_) => Ok(()),
        }
    }

    fn catalog(&self) -> Result<Value, String> {
        let mut resp = agent().get(CATALOG).call().map_err(|err| err.to_string())?;
        let raw = resp
            .body_mut()
            .with_config()
            .limit(64 << 20) // 3 MB in 2026; ureq's default stops at 10
            .read_to_string()
            .map_err(|err| err.to_string())?;
        serde_json::from_str(&raw).map_err(|err| format!("price catalog: {err}"))
    }
}

/// A fake release: one tag, one asset body, optionally cut short.
#[cfg(test)]
pub(crate) struct FakeReleases {
    pub(crate) tag: String,
    pub(crate) body: Vec<u8>,
    /// The download writes half the body and fails, as a short body does.
    pub(crate) truncated: bool,
    pub(crate) calls: std::sync::atomic::AtomicUsize,
    /// The price catalog; null prices nothing.
    pub(crate) catalog: Value,
}

#[cfg(test)]
impl FakeReleases {
    pub(crate) fn new(tag: &str, body: &[u8]) -> Self {
        FakeReleases {
            tag: tag.to_string(),
            body: body.to_vec(),
            truncated: false,
            calls: std::sync::atomic::AtomicUsize::new(0),
            catalog: Value::Null,
        }
    }
}

/// A release body: the target's magic, then `rest`.
#[cfg(test)]
pub(crate) fn binary(rest: &[u8]) -> Vec<u8> {
    [&TARGET.unwrap().1[..], rest].concat()
}

#[cfg(test)]
impl Releases for FakeReleases {
    fn latest_tag(&self) -> Result<String, String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.tag.clone())
    }

    fn download(&self, tag: &str, target: &str, dest: &Path) -> Result<(), String> {
        assert_eq!(tag, self.tag);
        assert_eq!(Some(target), TARGET.map(|(t, _)| t));
        if self.truncated {
            fs::write(dest, &self.body[..self.body.len() / 2]).unwrap();
            return Err(format!(
                "truncated body: {} of {} bytes",
                self.body.len() / 2,
                self.body.len()
            ));
        }
        fs::write(dest, &self.body).map_err(|err| err.to_string())
    }

    fn catalog(&self) -> Result<Value, String> {
        Ok(self.catalog.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::write_file;
    use crate::tempdir::TempDir;
    use crate::tools::fake::Fake;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    /// A scratch exe standing in for the running binary; never the test binary.
    fn exe(dir: &TempDir) -> PathBuf {
        let exe = dir.path().join("orqa");
        fs::write(&exe, b"old").unwrap();
        exe
    }

    fn temp_files(dir: &TempDir) -> Vec<String> {
        fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".new."))
            .collect()
    }

    #[test]
    fn a_greater_version_downloads_and_swaps() {
        let dir = TempDir::new();
        let exe = exe(&dir);
        fs::write(dir.path().join("orqa.new.1"), b"left by a dead process").unwrap();
        let marker = dir.path().join("orqa.updated");
        fs::write(&marker, "v1.0.0").unwrap();
        let releases = FakeReleases::new("v1.1.0", &binary(b"new"));
        let ready = check(&releases, "v1.0.0", &exe)
            .unwrap()
            .expect("no update");
        assert_eq!(ready.tag, "v1.1.0");
        assert_eq!(fs::read(&exe).unwrap(), b"old", "swapped before install");
        assert_eq!(fs::read(&marker).unwrap(), b"v1.0.0", "the sweep took it");
        assert_eq!(
            temp_files(&dir),
            [format!("orqa.new.{}", std::process::id())]
        );
        let mode = fs::metadata(&ready.tmp).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "mode {mode:o}");
        ready.install().unwrap();
        assert_eq!(fs::read(&exe).unwrap(), binary(b"new"));
        assert!(temp_files(&dir).is_empty());
        assert_eq!(fs::read(&marker).unwrap(), b"v1.1.0");
    }

    #[test]
    fn an_equal_or_lesser_version_does_not_download() {
        let dir = TempDir::new();
        let exe = exe(&dir);
        for tag in ["v1.0.0", "v0.9.9", "v0.10.0"] {
            let releases = FakeReleases::new(tag, b"new");
            assert!(check(&releases, "v1.0.0", &exe).unwrap().is_none(), "{tag}");
        }
        assert!(check(
            &FakeReleases::new("v1.10.0", &binary(b"new")),
            "v1.9.0",
            &exe
        )
        .unwrap()
        .is_some());
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert_eq!(
            check(&FakeReleases::new("latest", b""), "v1.0.0", &exe).unwrap_err(),
            "unreadable tag \"latest\""
        );
    }

    #[test]
    fn a_truncated_body_leaves_no_temp_file() {
        let dir = TempDir::new();
        let exe = exe(&dir);
        let mut releases = FakeReleases::new("v1.1.0", &binary(b"new binary"));
        releases.truncated = true;
        assert_eq!(
            check(&releases, "v1.0.0", &exe).unwrap_err(),
            "truncated body: 7 of 14 bytes"
        );
        assert!(temp_files(&dir).is_empty(), "{:?}", temp_files(&dir));
        assert_eq!(fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn a_refused_rename_reports_and_drops_the_temp_file() {
        let dir = TempDir::new();
        let exe = exe(&dir);
        let ready = check(
            &FakeReleases::new("v1.1.0", &binary(b"new")),
            "v1.0.0",
            &exe,
        )
        .unwrap()
        .unwrap();
        fs::remove_file(&exe).unwrap();
        fs::create_dir(&exe).unwrap(); // rename of a file over a directory is refused
        let err = ready.install().unwrap_err();
        assert!(err.starts_with("rename refused: "), "{err}");
        assert!(temp_files(&dir).is_empty());
        assert!(!dir.path().join("orqa.updated").exists());
    }

    /// The marker goes whatever it names; only the running version's
    /// counts.
    #[test]
    fn taking_the_marker_returns_the_running_version_and_deletes_it() {
        let dir = TempDir::new();
        let exe = exe(&dir);
        let marker = dir.path().join("orqa.updated");
        fs::write(&marker, "v1.1.0").unwrap();
        assert!(take_marker(&exe, "v1.1.0"));
        assert!(!marker.exists());
        fs::write(&marker, "v1.1.0").unwrap();
        assert!(!take_marker(&exe, "v1.0.0"), "an undone install");
        assert!(!marker.exists());
        assert!(!take_marker(&exe, "v1.1.0"), "no marker");
    }

    #[test]
    fn a_file_that_is_not_the_targets_binary_is_refused() {
        let dir = TempDir::new();
        let exe = exe(&dir);
        let releases = FakeReleases::new("v1.1.0", b"<!DOCTYPE html>");
        assert_eq!(
            check(&releases, "v1.0.0", &exe).unwrap_err(),
            format!("not a {} binary", TARGET.unwrap().0)
        );
        assert!(temp_files(&dir).is_empty(), "{:?}", temp_files(&dir));
        assert_eq!(fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn a_dev_build_never_checks() {
        let dir = TempDir::new();
        let exe = exe(&dir);
        let releases = FakeReleases::new("v9.0.0", &binary(b"new"));
        for version in ["v1.0.0-dev", "1.0.0", "v1.0"] {
            assert!(check(&releases, version, &exe).unwrap().is_none());
        }
        assert_eq!(
            releases.calls.load(Ordering::SeqCst),
            0,
            "a dev build asked"
        );
        assert!(temp_files(&dir).is_empty());
    }

    /// A catalog entry at $1 in and $5 out per million tokens.
    fn priced() -> Value {
        json!({"input_cost_per_token": 1e-06, "output_cost_per_token": 5e-06})
    }

    fn kept(repo: &Path) -> Vec<String> {
        let raw = fs::read_to_string(prices_file(repo)).unwrap_or_default();
        let doc: serde_json::Map<String, Value> = serde_json::from_str(&raw).unwrap_or_default();
        doc.keys().cloned().collect()
    }

    /// init's prices.json: the rows' models and every model their Apps on
    /// PATH list, the ones the catalog prices, made once. A dev build asks
    /// nothing.
    #[test]
    fn init_keeps_the_prices_of_the_models_its_apps_list_once() {
        let repo = TempDir::new();
        let config = json!({
            "implement": {"model": "claude-opus-5-5[1m]"},
            "review": {"model": "gpt-6.1-sol"},
        });
        write_file(
            &repo.path().join(".orqadence/config.json"),
            &config.to_string(),
        );
        let tools = Fake::new(|_, argv| match argv {
            ["which", "claude" | "codex"] => Ok(String::new()),
            ["codex", "debug", "models", "--bundled"] => Ok(json!({"models": [
                {"slug": "gpt-6-sol", "visibility": "list"},
                {"slug": "gpt-6-hidden", "visibility": "hide"},
            ]})
            .to_string()),
            _ => Err(format!("unexpected {argv:?}")),
        });
        let mut releases = FakeReleases::new("v1.0.0", b"");
        let names = [
            "claude-opus-5-5",
            "claude-haiku-4-5",
            "gpt-6.1-sol",
            "gpt-6-sol",
            "gpt-6-hidden",
            "gpt-6-astra",
        ];
        releases.catalog = names.iter().map(|n| (n.to_string(), priced())).collect();
        let mut out = Vec::new();
        seed_prices(repo.path(), &*tools, &releases, "v1.0.0-dev", &mut out);
        assert!(out.is_empty() && !prices_file(repo.path()).exists());

        seed_prices(repo.path(), &*tools, &releases, "v1.0.0", &mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "init: prices of 4 models kept in .orqadence-local/prices.json\n"
        );
        let four = [
            "claude-haiku-4-5",
            "claude-opus-5-5",
            "gpt-6-sol",
            "gpt-6.1-sol",
        ];
        assert_eq!(kept(repo.path()), four);

        releases.catalog = json!({"gpt-6-astra": priced()});
        let mut out = Vec::new();
        write_file(
            &repo.path().join(".orqadence/config.json"),
            r#"{"review": {"model": "gpt-6-astra"}}"#,
        );
        seed_prices(repo.path(), &*tools, &releases, "v1.0.0", &mut out);
        assert!(out.is_empty(), "made twice");
        assert_eq!(kept(repo.path()), four);
    }

    /// A check keeps the price of a model a row changed to, and the models
    /// kept before stay. A dev build asks nothing.
    #[test]
    fn a_check_adds_the_price_of_a_model_a_row_changed_to() {
        let repo = TempDir::new();
        let config = repo.path().join(".orqadence/config.json");
        let mut releases = FakeReleases::new("v1.0.0", b"");
        releases.catalog = json!({
            "claude-fable-5-1": priced(),
            "gpt-6-sol": priced(),
            "gpt-6.1-sol": priced(),
        });
        write_file(
            &config,
            r#"{"review": {"model": "gpt-6-sol"}, "fix": {"model": "fable"}}"#,
        );
        refresh_prices(&releases, "v1.0.0", repo.path()).unwrap();
        assert_eq!(kept(repo.path()), ["claude-fable-5-1", "gpt-6-sol"]);

        write_file(&config, r#"{"review": {"model": "gpt-6.1-sol"}}"#);
        refresh_prices(&releases, "v1.0.0-dev", repo.path()).unwrap();
        assert_eq!(kept(repo.path()), ["claude-fable-5-1", "gpt-6-sol"]);
        refresh_prices(&releases, "v1.0.0", repo.path()).unwrap();
        assert_eq!(
            kept(repo.path()),
            ["claude-fable-5-1", "gpt-6-sol", "gpt-6.1-sol"]
        );
    }
}
