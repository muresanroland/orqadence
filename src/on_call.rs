//! On call (harness-we9): the Moshi doorbell that pushes to the phone, and
//! the per-person On call settings in .orqadence-local/config.json.

use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use crate::orchestrator::state::local_dir;

/// The per-person settings file (ADR 0006), On call under "on_call".
const CONFIG: &str = ".orqadence-local/config.json";
/// Wins over the file's token, as TYPESAFE_API_KEY wins over the kept key.
const TOKEN_VAR: &str = "MOSHI_WEBHOOK_TOKEN";
const DEFAULT_MINUTES: u64 = 5;

/// The On call settings, loaded at Screen::open. No token is off: there is
/// no separate switch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OnCall {
    pub(crate) token: Option<String>,
    /// The minutes nobody answers before the Shell goes On call.
    pub(crate) minutes: u64,
}

impl Default for OnCall {
    fn default() -> Self {
        OnCall {
            token: None,
            minutes: DEFAULT_MINUTES,
        }
    }
}

/// The settings: MOSHI_WEBHOOK_TOKEN when set, else the file's token; the
/// file's minutes when a whole number of at least 1, else 5. A file that is
/// missing or unreadable is the defaults.
pub(crate) fn load(repo: &Path, env: &dyn Fn(&str) -> String) -> OnCall {
    let doc: Value = fs::read_to_string(repo.join(CONFIG))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let on_call = &doc["on_call"];
    let from_env = env(TOKEN_VAR);
    let token = match from_env.trim() {
        "" => on_call["token"].as_str().unwrap_or_default().trim(),
        token => token,
    };
    OnCall {
        token: (!token.is_empty()).then(|| token.to_string()),
        minutes: on_call["minutes"]
            .as_u64()
            .filter(|m| *m >= 1)
            .unwrap_or(DEFAULT_MINUTES),
    }
}

/// Keeps the settings in the per-person config.json, its other keys as they
/// were, readable only by the user since it holds the token.
#[allow(dead_code)] // /config's On call page (harness-we9.3) saves
pub(crate) fn save(repo: &Path, on_call: &OnCall) -> io::Result<()> {
    local_dir(repo)?;
    let path = repo.join(CONFIG);
    let mut doc: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    doc["on_call"] = json!({"token": on_call.token, "minutes": on_call.minutes});
    let mut file = File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    // mode() only sets a new file's; one made otherwise is narrowed too.
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(format!("{doc:#}\n").as_bytes())
}

const URL: &str = "https://api.getmoshi.app/api/webhook";
/// A push is one small POST; the Shell never waits long on it.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The seam to the phone: one push. An error never carries the token.
pub(crate) trait Doorbell: Send + Sync {
    #[allow(dead_code)] // the On call state (harness-we9.2) rings
    fn ring(&self, token: &str, title: &str, message: &str) -> Result<(), String>;
}

/// The real Doorbell: Moshi's webhook, over ureq. ureq's own User-Agent
/// stays: Moshi's Cloudflare refuses some defaults (403, error 1010).
pub(crate) struct Moshi;

impl Doorbell for Moshi {
    fn ring(&self, token: &str, title: &str, message: &str) -> Result<(), String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        // A non-2xx status is an Err too: ureq's http_status_as_error.
        agent
            .post(URL)
            .header("Content-Type", "application/json")
            .send(body(token, title, message))
            .map(|_| ())
            .map_err(|err| scrub(&err.to_string(), token))
    }
}

/// A Doorbell for tests: records every ring, and fails each while `fail`.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct FakeDoorbell {
    pub(crate) rings: std::sync::Mutex<Vec<(String, String, String)>>,
    pub(crate) fail: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl Doorbell for FakeDoorbell {
    fn ring(&self, token: &str, title: &str, message: &str) -> Result<(), String> {
        let ring = (token.to_string(), title.to_string(), message.to_string());
        self.rings.lock().unwrap().push(ring);
        match self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            true => Err("http status: 500".to_string()),
            false => Ok(()),
        }
    }
}

/// The webhook's JSON body.
pub(crate) fn body(token: &str, title: &str, message: &str) -> String {
    json!({"token": token, "title": title, "message": message}).to_string()
}

/// A failure's text with the token redacted, as tools.rs's command_line
/// redacts the TypeSafe key.
fn scrub(err: &str, token: &str) -> String {
    if token.is_empty() {
        return err.to_string();
    }
    err.replace(token, "***")
}

#[cfg(test)]
mod on_call_test;
