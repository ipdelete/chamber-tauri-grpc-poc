mod protocol {
    tonic::include_proto!("chamber.agent.v1");
}

use std::fmt::Write;

use crate::lens::LensDefinition;

use protocol::agent_event::Payload;
use protocol::agent_runtime_client::AgentRuntimeClient;
use protocol::host_message::Payload as HostPayload;
use protocol::approval_decision::Outcome;
use protocol::{Approved, ApprovalDecision, Denied, HostMessage, UserPrompt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, transport::Channel};

pub fn generate_auth_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(token, "{byte:02x}").expect("writing to a string cannot fail");
    }
    Ok(token)
}

#[derive(Debug, PartialEq)]
pub struct ChatEvent {
    pub session_id: String,
    pub payload: ChatEventPayload,
}

#[derive(Debug, PartialEq)]
pub enum ChatEventPayload {
    Started,
    TextDelta(String),
    Completed,
    RuntimeError {
        code: String,
        message: String,
        retryable: bool,
    },
    ApprovalRequest {
        tool_call_id: String,
        tool_name: String,
        arguments_json: String,
    },
    LensChanged(LensDefinition),
}

#[derive(Clone)]
pub struct AgentRuntime {
    client: AgentRuntimeClient<Channel>,
    auth_token: String,
}

impl AgentRuntime {
    pub async fn connect(
        endpoint: impl Into<String>,
        auth_token: impl Into<String>,
    ) -> Result<Self, tonic::transport::Error> {
        let client = AgentRuntimeClient::connect(endpoint.into()).await?;
        Ok(Self {
            client,
            auth_token: auth_token.into(),
        })
    }

    pub async fn interactive_chat(
        &mut self,
        session_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<ChatStream, tonic::Status> {
        let session_id = session_id.into();
        let (requests, incoming) = mpsc::channel(8);
        requests
            .send(HostMessage {
                session_id: session_id.clone(),
                payload: Some(HostPayload::Prompt(UserPrompt {
                    text: prompt.into(),
                })),
            })
            .await
            .map_err(|_| tonic::Status::unavailable("agent request stream closed"))?;

        let mut request = Request::new(ReceiverStream::new(incoming));
        self.authenticate(&mut request)?;
        let response = self.client.interact(request).await?;

        Ok(ChatStream {
            inner: response.into_inner(),
            requests: Some(requests),
            session_id,
        })
    }

    fn authenticate<T>(&self, request: &mut Request<T>) -> Result<(), tonic::Status> {
        request.metadata_mut().insert(
            "x-chamber-token",
            self.auth_token
                .parse()
                .map_err(|_| tonic::Status::internal("invalid sidecar authentication token"))?,
        );
        Ok(())
    }
}

pub struct ChatStream {
    inner: tonic::Streaming<protocol::AgentEvent>,
    requests: Option<mpsc::Sender<HostMessage>>,
    session_id: String,
}

impl ChatStream {
    pub async fn message(&mut self) -> Result<Option<ChatEvent>, tonic::Status> {
        let Some(event) = self.inner.message().await? else {
            return Ok(None);
        };
        let payload = match event.payload {
            Some(Payload::Started(_)) => ChatEventPayload::Started,
            Some(Payload::TextDelta(delta)) => ChatEventPayload::TextDelta(delta.text),
            Some(Payload::Completed(_)) => ChatEventPayload::Completed,
            Some(Payload::Error(error)) => ChatEventPayload::RuntimeError {
                code: error.code,
                message: error.message,
                retryable: error.retryable,
            },
            Some(Payload::ApprovalRequest(request)) => ChatEventPayload::ApprovalRequest {
                tool_call_id: request.tool_call_id,
                tool_name: request.tool_name,
                arguments_json: request.arguments_json,
            },
            Some(Payload::LensChanged(lens)) => ChatEventPayload::LensChanged(LensDefinition {
                id: lens.id,
                name: lens.name,
                icon: lens.icon,
                html: lens.html,
            }),
            None => return Err(tonic::Status::data_loss("agent event has no payload")),
        };

        Ok(Some(ChatEvent {
            session_id: event.session_id,
            payload,
        }))
    }

    pub async fn send_approval_decision(
        &self,
        tool_call_id: String,
        decision: Result<(), String>,
    ) -> Result<(), tonic::Status> {
        let requests = self
            .requests
            .as_ref()
            .ok_or_else(|| tonic::Status::failed_precondition("chat is not interactive"))?;
        let outcome = match decision {
            Ok(()) => Outcome::Approved(Approved {}),
            Err(reason) => Outcome::Denied(Denied { reason }),
        };
        requests
            .send(HostMessage {
                session_id: self.session_id.clone(),
                payload: Some(HostPayload::ApprovalDecision(ApprovalDecision {
                    tool_call_id,
                    outcome: Some(outcome),
                })),
            })
            .await
            .map_err(|_| tonic::Status::unavailable("agent request stream closed"))
    }
}
