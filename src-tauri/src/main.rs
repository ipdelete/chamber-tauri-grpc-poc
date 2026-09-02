#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;

use chamber_tauri_host::agent_runtime::{AgentRuntime, ChatEventPayload};
use chamber_tauri_host::bundled_sidecar::BundledSidecarProcess;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex, oneshot};

struct AgentHost {
    runtime: AgentRuntime,
    sidecar: BundledSidecarProcess,
}

impl AgentHost {
    async fn start(app: &AppHandle) -> Result<Self, String> {
        let sidecar = BundledSidecarProcess::start(app).await?;
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
    active_requests: Mutex<HashMap<String, Option<oneshot::Sender<()>>>>,
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
    Cancelled {
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

    let (cancel, mut cancelled) = oneshot::channel();
    {
        let mut active_requests = state.active_requests.lock().await;
        if active_requests.contains_key(&session_id) {
            return Err(format!(
                "session {session_id:?} already has an active request"
            ));
        }
        active_requests.insert(session_id.clone(), Some(cancel));
    }

    let result = async {
        let mut runtime = {
            let mut host = state.host.lock().await;
            if host.is_none() {
                *host = Some(AgentHost::start(&app).await?);
            }
            host.as_ref()
                .expect("agent host was initialized")
                .runtime
                .clone()
        };
        let mut events = tokio::select! {
            _ = &mut cancelled => {
                app.emit("chat-event", ChatUiEvent::Cancelled { session_id: session_id.clone() })
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }
            result = runtime.chat(session_id.clone(), prompt) => {
                result.map_err(|error| error.to_string())?
            }
        };

        loop {
            let event = tokio::select! {
                _ = &mut cancelled => {
                    app.emit("chat-event", ChatUiEvent::Cancelled { session_id: session_id.clone() })
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                result = events.message() => {
                    let Some(event) = result.map_err(|error| error.to_string())? else {
                        return Err("chat stream ended without a completed event".to_owned());
                    };
                    event
                }
            };

            let terminal = matches!(
                event.payload,
                ChatEventPayload::Completed | ChatEventPayload::RuntimeError { .. }
            );
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
            if terminal {
                return Ok(());
            }
        }
    }
    .await;

    state.active_requests.lock().await.remove(&session_id);
    result
}

#[tauri::command]
async fn cancel_message(state: State<'_, AgentState>, session_id: String) -> Result<(), String> {
    let mut active_requests = state.active_requests.lock().await;
    let cancel = active_requests
        .get_mut(&session_id)
        .ok_or_else(|| format!("session {session_id:?} has no active request"))?
        .take()
        .ok_or_else(|| format!("session {session_id:?} is already cancelling"))?;
    cancel
        .send(())
        .map_err(|_| format!("session {session_id:?} already finished"))
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AgentState::default())
        .invoke_handler(tauri::generate_handler![send_message, cancel_message])
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
