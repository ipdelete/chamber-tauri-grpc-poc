use std::path::Path;
use std::time::Duration;

use crate::agent_runtime::generate_auth_token;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tokio::time::timeout;

pub struct BundledSidecarProcess {
    child: CommandChild,
    events: tauri::async_runtime::Receiver<CommandEvent>,
    port: u16,
    auth_token: String,
}

impl BundledSidecarProcess {
    pub async fn start(app: &AppHandle, mind_root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(mind_root).map_err(|error| error.to_string())?;
        let mind_root = mind_root
            .to_str()
            .ok_or_else(|| "mind root is not valid UTF-8".to_owned())?;
        let command = app
            .shell()
            .sidecar("chamber-agent-sidecar")
            .map_err(|error| error.to_string())?
            .args(["--port", "0", "--shutdown-on-stdin", "--mind-root", mind_root]);
        let (mut events, mut child) = command.spawn().map_err(|error| error.to_string())?;
        let auth_token = generate_auth_token().map_err(|error| error.to_string())?;
        child
            .write(format!("AUTH {auth_token}\n").as_bytes())
            .map_err(|error| error.to_string())?;

        let port = timeout(Duration::from_secs(30), async {
            loop {
                match events.recv().await {
                    Some(CommandEvent::Stdout(line)) => {
                        let line = String::from_utf8(line).map_err(|error| error.to_string())?;
                        if let Some(port) = line.trim().strip_prefix("READY ") {
                            return port.parse::<u16>().map_err(|error| error.to_string());
                        }
                        return Err(format!("invalid sidecar readiness: {line:?}"));
                    }
                    Some(CommandEvent::Stderr(line)) => {
                        eprintln!("agent sidecar: {}", String::from_utf8_lossy(&line));
                    }
                    Some(CommandEvent::Error(error)) => return Err(error),
                    Some(CommandEvent::Terminated(status)) => {
                        return Err(format!(
                            "sidecar exited before becoming ready: {:?}",
                            status.code
                        ));
                    }
                    None => return Err("sidecar closed before becoming ready".to_owned()),
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| "sidecar did not become ready within 30 seconds".to_owned())??;

        Ok(Self {
            child,
            events,
            port,
            auth_token,
        })
    }

    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    pub async fn shutdown(self) -> Result<(), String> {
        let Self {
            mut child,
            mut events,
            ..
        } = self;
        child
            .write(b"SHUTDOWN\n")
            .map_err(|error| error.to_string())?;

        let terminated = timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Some(CommandEvent::Terminated(status)) => return Ok(status.code),
                    Some(CommandEvent::Stderr(line)) => {
                        eprintln!("agent sidecar: {}", String::from_utf8_lossy(&line));
                    }
                    Some(CommandEvent::Error(error)) => return Err(error),
                    None => return Err("sidecar event stream closed".to_owned()),
                    _ => {}
                }
            }
        })
        .await;

        match terminated {
            Ok(Ok(Some(0))) => Ok(()),
            Ok(Ok(code)) => Err(format!("sidecar exited with status {code:?}")),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                child.kill().map_err(|error| error.to_string())?;
                Err("sidecar did not shut down within 10 seconds".to_owned())
            }
        }
    }
}
