use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::timeout;

pub struct SidecarProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    port: u16,
}

impl SidecarProcess {
    pub async fn start() -> io::Result<Self> {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or_else(|| io::Error::other("Rust manifest has no parent directory"))?
            .to_owned();
        let sidecar_project = project_root.join("sidecar");
        let server = sidecar_project.join("src/server.py");
        let mut child = Command::new("uv")
            .arg("run")
            .arg("--project")
            .arg(&sidecar_project)
            .arg("python")
            .arg(server)
            .arg("--port")
            .arg("0")
            .arg("--shutdown-on-stdin-eof")
            .current_dir(project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("sidecar stdin was not captured"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("sidecar stdout was not captured"))?;
        let mut reader = BufReader::new(stdout);
        let mut readiness = String::new();

        let bytes_read = timeout(Duration::from_secs(30), reader.read_line(&mut readiness))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "sidecar did not become ready")
            })??;
        if bytes_read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "sidecar exited before becoming ready",
            ));
        }

        let port = readiness
            .trim()
            .strip_prefix("READY ")
            .ok_or_else(|| io::Error::other(format!("invalid sidecar readiness: {readiness:?}")))?
            .parse()
            .map_err(|error| io::Error::other(format!("invalid sidecar port: {error}")))?;

        Ok(Self {
            child,
            stdin: Some(stdin),
            port,
        })
    }

    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        drop(self.stdin.take());

        match timeout(Duration::from_secs(10), self.child.wait()).await {
            Ok(status) => {
                let status = status?;
                if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!("sidecar exited with {status}")))
                }
            }
            Err(_) => {
                self.child.kill().await?;
                self.child.wait().await?;
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "sidecar did not shut down gracefully",
                ))
            }
        }
    }
}
