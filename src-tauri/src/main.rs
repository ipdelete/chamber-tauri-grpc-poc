#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use chamber_tauri_host::agent_runtime::{AgentRuntime, ChatEventPayload};
use chamber_tauri_host::sidecar::SidecarProcess;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

struct AgentHost {
    runtime: AgentRuntime,
    sidecar: SidecarProcess,
}

impl AgentHost {
    async fn start() -> Result<Self, String> {
        let sidecar = SidecarProcess::start()
            .await
            .map_err(|error| error.to_string())?;
        let runtime = AgentRuntime::connect(sidecar.endpoint())
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self { runtime, sidecar })
    }

    async fn shutdown(self) -> Result<(), String> {
        drop(self.runtime);
        self.sidecar
            .shutdown()
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Default)]
struct AgentState {
    host: Mutex<Option<AgentHost>>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatUiEvent {
    Started {
        session_id: String,
    },
    TextDelta {
        session_id: String,
        text: String,
    },
    Completed {
        session_id: String,
    },
    Error {
        session_id: String,
        code: String,
        message: String,
        retryable: bool,
    },
}

#[tauri::command]
async fn send_message(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
    prompt: String,
) -> Result<(), String> {
    if session_id.trim().is_empty() {
        return Err("session_id cannot be empty".to_owned());
    }
    if prompt.trim().is_empty() {
        return Err("prompt cannot be empty".to_owned());
    }

    let mut host = state.host.lock().await;
    if host.is_none() {
        *host = Some(AgentHost::start().await?);
    }
    let runtime = &mut host.as_mut().expect("agent host was initialized").runtime;
    let mut events = runtime
        .chat(session_id, prompt)
        .await
        .map_err(|error| error.to_string())?;

    while let Some(event) = events.message().await.map_err(|error| error.to_string())? {
        let payload = match event.payload {
            ChatEventPayload::Started => ChatUiEvent::Started {
                session_id: event.session_id,
            },
            ChatEventPayload::TextDelta(text) => ChatUiEvent::TextDelta {
                session_id: event.session_id,
                text,
            },
            ChatEventPayload::Completed => ChatUiEvent::Completed {
                session_id: event.session_id,
            },
            ChatEventPayload::RuntimeError {
                code,
                message,
                retryable,
            } => ChatUiEvent::Error {
                session_id: event.session_id,
                code,
                message,
                retryable,
            },
        };
        app.emit("chat-event", payload)
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn main() {
    let app = tauri::Builder::default()
        .manage(AgentState::default())
        .invoke_handler(tauri::generate_handler![send_message])
        .build(tauri::generate_context!())
        .expect("failed to build Chamber");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            let state = app_handle.state::<AgentState>();
            tauri::async_runtime::block_on(async {
                if let Some(host) = state.host.lock().await.take() {
                    if let Err(error) = host.shutdown().await {
                        eprintln!("failed to stop agent sidecar: {error}");
                    }
                }
            });
        }
    });
}
