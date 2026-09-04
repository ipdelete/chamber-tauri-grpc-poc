use std::path::PathBuf;
use std::pin::Pin;

use futures::StreamExt;
use rig::client::CompletionClient;
use rig::completion::{CompletionModel, ToolDefinition};
use rig::message::{ToolResultContent, UserContent};
use rig::providers::openai;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::bridge::{ApprovalBridge, ApprovalState};
use crate::lens;
use crate::proto::{self, agent_runtime_server::AgentRuntime};

const DEFAULT_BASE_URL: &str = "http://bigbertha:11434/v1";
const DEFAULT_MODEL: &str = "glm-5.3-flash:cloud";

const LENS_INSTRUCTIONS: &str = "You are an agent inside Chamber. When the user asks for a dashboard, view, \
panel, report, form, command center, or change to the current UI, use the \
lens_upsert tool. Create a complete, self-contained HTML document. Use a \
short lowercase ID containing only letters, numbers, and hyphens. Canvas \
buttons may call window.canvas.sendAction(action, data) to send intent back \
to you. Do not use external scripts, styles, images, or network requests. \
Use the Chamber classes ch-page, ch-grid, ch-card, ch-button, \
ch-button-secondary, ch-input, ch-table, ch-badge, and ch-muted.";

#[derive(Clone)]
pub struct RuntimeServer {
    auth_token: String,
    mind_root: PathBuf,
    client: openai::CompletionsClient,
    model_name: String,
}

impl RuntimeServer {
    pub fn new(auth_token: String, mind_root: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model_name = std::env::var("CHAMBER_MODEL")
            .unwrap_or_else(|_| DEFAULT_MODEL.to_string());

        let client = openai::CompletionsClient::builder()
            .api_key("ollama")
            .base_url(base_url)
            .build()?;

        Ok(Self {
            auth_token,
            mind_root,
            client,
            model_name,
        })
    }

    fn authenticate(&self, request: &Request<Streaming<proto::HostMessage>>) -> Result<(), Status> {
        let token_opt = request
            .metadata()
            .get("x-chamber-token")
            .and_then(|v| v.to_str().ok());

        let authed = match token_opt {
            Some(t) => bool::from(t.as_bytes().ct_eq(self.auth_token.as_bytes())),
            None => false,
        };

        if !authed {
            return Err(Status::unauthenticated("Invalid sidecar authentication token"));
        }

        Ok(())
    }

    async fn run_agent(
        bridge: ApprovalBridge,
        mind_root: PathBuf,
        model: <openai::CompletionsClient as CompletionClient>::CompletionModel,
        prompt_text: String,
    ) {
        if bridge
            .emit(proto::agent_event::Payload::Started(proto::Started {}))
            .await
            .is_err()
        {
            return;
        }

        let mut chat_history: Vec<rig::completion::Message> = Vec::new();
        let mut current_prompt = rig::completion::Message::user(prompt_text);

        let tool_def = ToolDefinition {
            name: "lens_upsert".to_string(),
            description: "Create or replace a sandboxed Canvas Lens in Chamber.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Short lowercase ID containing only letters, numbers, and hyphens" },
                    "name": { "type": "string", "description": "Human readable name of the lens" },
                    "icon": { "type": "string", "description": "Icon name for the lens" },
                    "html": { "type": "string", "description": "Complete HTML document" }
                },
                "required": ["id", "name", "icon", "html"]
            }),
        };

        loop {
            let req = model
                .completion_request(current_prompt.clone())
                .messages(chat_history.clone())
                .preamble(LENS_INSTRUCTIONS.to_string())
                .tool(tool_def.clone())
                .build();

            let mut stream = match model.stream(req).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = bridge
                        .emit(proto::agent_event::Payload::Error(proto::RuntimeError {
                            code: "StreamError".to_string(),
                            message: e.to_string(),
                            retryable: false,
                        }))
                        .await;
                    return;
                }
            };

            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(rig::streaming::StreamedAssistantContent::Text(text)) => {
                        if !text.text.is_empty() {
                            let emit_res = bridge
                                .emit(proto::agent_event::Payload::TextDelta(proto::TextDelta {
                                    text: text.text,
                                }))
                                .await;
                            if emit_res.is_err() {
                                return;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = bridge
                            .emit(proto::agent_event::Payload::Error(proto::RuntimeError {
                                code: "StreamChunkError".to_string(),
                                message: e.to_string(),
                                retryable: false,
                            }))
                            .await;
                        return;
                    }
                }
            }

            let tool_calls: Vec<rig::message::ToolCall> = stream
                .choice
                .iter()
                .filter_map(|c| {
                    if let rig::message::AssistantContent::ToolCall(tc) = c {
                        Some(tc.clone())
                    } else {
                        None
                    }
                })
                .collect();

            if tool_calls.is_empty() {
                let _ = bridge
                    .emit(proto::agent_event::Payload::Completed(proto::Completed {}))
                    .await;
                break;
            }

            chat_history.push(current_prompt);
            chat_history.push(rig::completion::Message::Assistant {
                id: stream.message_id.clone(),
                content: stream.choice,
            });

            let mut tool_results = Vec::new();
            for tool_call in tool_calls {
                let tool_name = tool_call.function.name.clone();
                let tool_call_id = tool_call.wire_call_id().to_string();
                let args_val = tool_call.function.arguments.clone();
                let args_json = serde_json::to_string(&args_val).unwrap_or_default();

                let decision = match bridge
                    .request_approval(tool_call_id.clone(), tool_name.clone(), args_json)
                    .await
                {
                    Ok(d) => d,
                    Err(_) => return,
                };

                match decision {
                    Ok(()) => {
                        let id = args_val
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let name = args_val
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let icon = args_val
                            .get("icon")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let html = args_val
                            .get("html")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();

                        if let Err(err_msg) = lens::validate(id, name, icon, html) {
                            let _ = bridge
                                .emit(proto::agent_event::Payload::Error(proto::RuntimeError {
                                    code: "LensValidationError".to_string(),
                                    message: err_msg,
                                    retryable: false,
                                }))
                                .await;
                            return;
                        }

                        if let Err(e) = lens::write(&mind_root, id, name, icon, html) {
                            let _ = bridge
                                .emit(proto::agent_event::Payload::Error(proto::RuntimeError {
                                    code: "LensWriteError".to_string(),
                                    message: e.to_string(),
                                    retryable: false,
                                }))
                                .await;
                            return;
                        }

                        let emit_lens = bridge
                            .emit(proto::agent_event::Payload::LensChanged(
                                proto::LensChanged {
                                    id: id.to_string(),
                                    name: name.to_string(),
                                    icon: icon.to_string(),
                                    html: html.to_string(),
                                },
                            ))
                            .await;
                        if emit_lens.is_err() {
                            return;
                        }

                        let tool_output = serde_json::json!({
                            "ok": true,
                            "id": id,
                            "message": "Lens saved and displayed"
                        })
                        .to_string();

                        tool_results.push(UserContent::tool_result_for(
                            tool_call.id.clone(),
                            tool_call.provider.clone(),
                            tool_name,
                            vec![ToolResultContent::text(tool_output)],
                        ));
                    }
                    Err(reason) => {
                        let tool_output = format!("Tool call denied by user: {}", reason);
                        tool_results.push(UserContent::tool_result_for(
                            tool_call.id.clone(),
                            tool_call.provider.clone(),
                            tool_name,
                            vec![ToolResultContent::text(tool_output)],
                        ));
                    }
                }
            }

            current_prompt = rig::completion::Message::User {
                content: tool_results,
            };
        }
    }
}

#[tonic::async_trait]
impl AgentRuntime for RuntimeServer {
    type InteractStream =
        Pin<Box<dyn futures::Stream<Item = Result<proto::AgentEvent, Status>> + Send + 'static>>;

    async fn interact(
        &self,
        request: Request<Streaming<proto::HostMessage>>,
    ) -> Result<Response<Self::InteractStream>, Status> {
        self.authenticate(&request)?;

        let mut in_stream = request.into_inner();
        let first_msg = match in_stream.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => return Err(e),
            None => {
                return Err(Status::invalid_argument(
                    "Interact requires an initial prompt",
                ));
            }
        };

        let session_id = first_msg.session_id;
        let prompt_text = match first_msg.payload {
            Some(proto::host_message::Payload::Prompt(p)) => p.text,
            _ => {
                return Err(Status::invalid_argument(
                    "First Interact message must be a prompt",
                ));
            }
        };

        let approval_state = ApprovalState::default();
        let (events_tx, events_rx) = mpsc::channel(64);
        let bridge = ApprovalBridge::new(session_id, events_tx, approval_state.clone());

        let reader_state = approval_state.clone();
        let reader_handle = tokio::spawn(async move {
            while let Some(msg_res) = in_stream.next().await {
                match msg_res {
                    Ok(msg) => {
                        if let Some(proto::host_message::Payload::ApprovalDecision(decision)) =
                            msg.payload
                        {
                            reader_state.handle_decision(decision).await;
                        }
                    }
                    Err(_) => {
                        reader_state.cancel_all().await;
                        break;
                    }
                }
            }
        });

        let mind_root = self.mind_root.clone();
        let model = self.client.completion_model(self.model_name.clone());
        tokio::spawn(async move {
            Self::run_agent(bridge, mind_root, model, prompt_text).await;
            reader_handle.abort();
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(events_rx))))
    }
}
