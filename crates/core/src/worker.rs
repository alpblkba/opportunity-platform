//! Bridge to the browser worker.
//!
//! The worker is a separate Node process with its own dependency tree, spawned per operation and handed a JSON request on stdin. Keeping the browser out of this process is deliberate: a crash in Playwright cannot take the API down, and the worker's navigation gate is the only thing that decides what may be opened.

use std::path::PathBuf;
use std::process::Stdio;

use opportunity_domain::FormPlan;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// This deployment has no browser at all. Not a failure of an attempt: a capability that is absent by design.
    #[error("{0}")]
    Unavailable(String),
    #[error("could not start the browser worker: {0}")]
    Spawn(String),
    #[error("the browser worker failed: {0}")]
    Failed(String),
    #[error("the browser worker returned something unreadable: {0}")]
    Decode(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct FillValue {
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FillRequest {
    pub url: String,
    pub values: Vec<FillValue>,
    // The worker reads this as `documentPath`; the names must match or the document silently never attaches.
    #[serde(rename = "documentPath", skip_serializing_if = "Option::is_none")]
    pub document_path: Option<String>,
    pub reference: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FillResult {
    pub filled: Vec<String>,
    pub skipped: Vec<String>,
    pub screenshot: String,
    #[serde(rename = "payloadHash")]
    pub payload_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitRequest {
    #[serde(flatten)]
    pub fill: FillRequest,
    #[serde(rename = "authorizedPayloadHash")]
    pub authorized_payload_hash: String,
    /// The file the worker creates immediately before it clicks submit. Its existence is the only evidence left when the worker is killed, and it decides whether a retry is safe.
    #[serde(rename = "progressPath")]
    pub progress_path: String,
}

/// What the worker observed. Mirrors the three outcomes the worker can report and nothing else; there is deliberately no way to express "probably submitted".
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SubmitOutcome {
    Submitted {
        receipt: String,
        screenshot: String,
    },
    Unknown {
        diagnostic: String,
        screenshot: String,
    },
    Failed {
        diagnostic: String,
    },
}

/// Where the worker keeps its private artefacts. The same variable the worker itself reads, so the two agree about one directory rather than each guessing.
pub fn artefact_root() -> PathBuf {
    std::env::var("BROWSER_WORKER_ARTEFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("private/browser"))
}

/// The file the worker creates immediately before clicking submit, for one attempt.
///
/// Present after a worker died means the click happened and a submission may have left the machine. Absent means it did not. That difference is what separates an outcome that may be retried from one that must be reconciled by a person, so this file is the whole reason a crash is not simply a failure.
pub fn submit_marker(reference: &str) -> PathBuf {
    artefact_root().join(reference).join("submit-clicked")
}

/// Where the built worker lives. Configurable so a packaged install can put it elsewhere.
fn worker_entry() -> PathBuf {
    std::env::var("BROWSER_WORKER_ENTRY")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("apps/browser-worker/dist/cli.js"))
}

pub fn worker_available() -> bool {
    worker_entry().exists()
}

async fn run<T: for<'de> Deserialize<'de>>(
    command: &str,
    request: &impl Serialize,
) -> Result<T, WorkerError> {
    let entry = worker_entry();
    if !entry.exists() {
        // A deployment can legitimately ship without the browser, so this reads as the handoff it is rather than as a broken install.
        return Err(WorkerError::Unavailable(format!(
            "assisted application is not available in this deployment, so this one is yours to submit on the employer's own page. \
             (No browser worker at {}. Locally, build it with `npm --prefix apps/browser-worker run build`; in a container, run the image built from the `browser` target.)",
            entry.display()
        )));
    }
    let mut child = Command::new("node")
        .arg(&entry)
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| WorkerError::Spawn(e.to_string()))?;

    let body = serde_json::to_vec(request).map_err(|e| WorkerError::Decode(e.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&body)
            .await
            .map_err(|e| WorkerError::Failed(e.to_string()))?;
        stdin.shutdown().await.ok();
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| WorkerError::Failed(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // The worker reports its own errors as JSON so a navigation refusal reads as a refusal rather than as a crash.
    if let Ok(error) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if let Some(message) = error.get("error").and_then(|m| m.as_str()) {
            return Err(WorkerError::Failed(message.to_owned()));
        }
    }
    if !output.status.success() {
        return Err(WorkerError::Failed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    serde_json::from_str(&stdout).map_err(|e| WorkerError::Decode(format!("{e}: {stdout}")))
}

pub async fn inspect(url: &str) -> Result<FormPlan, WorkerError> {
    run("inspect", &serde_json::json!({ "url": url })).await
}

pub async fn fill(request: &FillRequest) -> Result<FillResult, WorkerError> {
    run("fill", request).await
}

pub async fn submit(request: &SubmitRequest) -> Result<SubmitOutcome, WorkerError> {
    run("submit", request).await
}
