use crate::{RuntimeBuilder, RuntimeError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

const DRIVER: &str = include_str!("browser_worker.py");

struct Worker {
    _child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
}

#[derive(Deserialize)]
struct Reply {
    value: Value,
    error: Option<String>,
}

impl Worker {
    fn start() -> Result<Self, String> {
        if std::env::var("CORVID_BROWSER_URL")
            .unwrap_or_default()
            .is_empty()
        {
            return Err("set CORVID_BROWSER_URL to the start page for this run".into());
        }
        let python = std::env::var_os("CORVID_BROWSER_PYTHON").unwrap_or_else(|| "python".into());
        let mut child = Command::new(python)
            .args(["-I", "-u", "-X", "utf8", "-c", DRIVER])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .kill_on_drop(true).spawn().map_err(|e| format!("start browser driver: {e}. Set CORVID_BROWSER_PYTHON to a Python installation with Playwright."))?;
        let input = child
            .stdin
            .take()
            .ok_or("browser driver stdin unavailable")?;
        let output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or("browser driver stdout unavailable")?,
        )
        .lines();
        Ok(Self {
            _child: child,
            input,
            output,
        })
    }

    async fn request(&mut self, name: &str, args: Vec<Value>) -> Result<Reply, String> {
        let mut line = json!({"tool": name, "args": args}).to_string();
        line.push('\n');
        self.input
            .write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let response = self
            .output
            .next_line()
            .await
            .map_err(|e| e.to_string())?
            .ok_or(
                "browser driver stopped; check the Playwright installation and browser channel",
            )?;
        serde_json::from_str(&response).map_err(|e| format!("invalid browser driver reply: {e}"))
    }
}

/// Install browser tools on a local CLI interpreter. Each builder owns its
/// own driver, created lazily on the first open. Replay uses recorded tools.
pub fn with_browser_tools(mut builder: RuntimeBuilder) -> RuntimeBuilder {
    let session = Arc::new(Mutex::new(None::<Worker>));
    for name in [
        "browser_open_scoped",
        "browser_read_text",
        "browser_click",
        "browser_capture",
        "browser_close",
    ] {
        let session = session.clone();
        builder = builder.tool(name, move |args| {
            let session = session.clone();
            async move {
                let fail = |message: String| RuntimeError::ToolFailed {
                    tool: name.into(),
                    message,
                };
                let mut slot = session.lock().await;
                let opening = name == "browser_open_scoped";
                if opening && slot.is_some() {
                    return Err(fail(
                        "browser already open; close it before opening another".into(),
                    ));
                }
                // Taking ownership keeps a cancelled/partial exchange from
                // leaving a stale reply for the next operation.
                let mut worker = if opening {
                    Worker::start().map_err(fail)?
                } else {
                    slot.take()
                        .ok_or_else(|| fail("open a browser first".into()))?
                };
                let reply =
                    tokio::time::timeout(Duration::from_secs(45), worker.request(name, args))
                        .await
                        .map_err(|_| fail("browser driver timed out".into()))?
                        .map_err(fail)?;
                if name != "browser_close" && (!opening || reply.error.is_none()) {
                    *slot = Some(worker);
                }
                match reply.error {
                    Some(message) => Err(fail(message)),
                    None => Ok(reply.value),
                }
            }
        });
    }
    builder
}
