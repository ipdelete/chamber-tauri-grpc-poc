use std::io::Write;
use std::path::PathBuf;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_stream::wrappers::TcpListenerStream;

mod bridge;
mod lens;
mod proto;
mod server;

use proto::agent_runtime_server::AgentRuntimeServer;
use server::RuntimeServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port: u16 = 0;
    let mut shutdown_on_stdin = false;
    let mut mind_root = PathBuf::from(".");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(p) = args.next() {
                    port = p.parse()?;
                }
            }
            "--shutdown-on-stdin" => {
                shutdown_on_stdin = true;
            }
            "--mind-root" => {
                if let Some(mr) = args.next() {
                    mind_root = PathBuf::from(mr);
                }
            }
            _ => {}
        }
    }

    // Read initial AUTH token from stdin
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut auth_line = String::new();
    let n = stdin.read_line(&mut auth_line).await?;
    if n == 0 {
        eprintln!("Unexpected EOF while reading AUTH token");
        std::process::exit(1);
    }

    let auth_token = match auth_line.trim().strip_prefix("AUTH ") {
        Some(t) => t.trim().to_string(),
        None => {
            eprintln!("Expected AUTH <token>, got: {}", auth_line.trim());
            std::process::exit(1);
        }
    };

    let bind_addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&bind_addr).await?;
    let bound_port = listener.local_addr()?.port();

    // Signal READY to host
    println!("READY {bound_port}");
    std::io::stdout().flush()?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    if shutdown_on_stdin {
        tokio::spawn(async move {
            let mut line = String::new();
            while let Ok(bytes) = stdin.read_line(&mut line).await {
                if bytes == 0 || line.trim() == "SHUTDOWN" {
                    let _ = shutdown_tx.send(());
                    break;
                }
                line.clear();
            }
        });
    }

    let incoming = TcpListenerStream::new(listener);
    let service = RuntimeServer::new(auth_token, mind_root)?;

    tonic::transport::Server::builder()
        .add_service(AgentRuntimeServer::new(service))
        .serve_with_incoming_shutdown(incoming, async {
            let _ = shutdown_rx.await;
        })
        .await?;

    Ok(())
}







