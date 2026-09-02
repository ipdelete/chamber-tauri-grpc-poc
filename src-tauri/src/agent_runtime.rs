mod protocol {
    tonic::include_proto!("chamber.agent.v1");
}

use std::fmt::Write;

use protocol::agent_event::Payload;
use protocol::agent_runtime_client::AgentRuntimeClient;
use protocol::host_message::Payload as HostPayload;
use protocol::host_tool_result::Outcome;
use protocol::{ChatRequest, HostMessage, HostToolResult, UserPrompt};
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
    HostToolCall {
        call_id: String,
        name: String,
        arguments_json: String,
    },
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

    pub async fn chat(
        &mut self,
        session_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<ChatStream, tonic::Status> {
        let session_id = session_id.into();
        let mut request = Request::new(ChatRequest {
            session_id: session_id.clone(),
            prompt: prompt.into(),
        });
        self.authenticate(&mut request)?;
        let response = self.client.chat(request).await?;

        Ok(ChatStream {
            inner: response.into_inner(),
            requests: None,
            session_id,
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
            Some(Payload::HostToolCall(call)) => ChatEventPayload::HostToolCall {
                call_id: call.call_id,
                name: call.name,
                arguments_json: call.arguments_json,
            },
            None => return Err(tonic::Status::data_loss("agent event has no payload")),
        };

        Ok(Some(ChatEvent {
            session_id: event.session_id,
            payload,
        }))
    }

    pub async fn send_tool_result(
        &self,
        call_id: String,
        result: Result<String, String>,
    ) -> Result<(), tonic::Status> {
        let requests = self
            .requests
            .as_ref()
            .ok_or_else(|| tonic::Status::failed_precondition("chat is not interactive"))?;
        let outcome = match result {
            Ok(json) => Outcome::ResultJson(json),
            Err(error) => Outcome::Error(error),
        };
        requests
            .send(HostMessage {
                session_id: self.session_id.clone(),
                payload: Some(HostPayload::ToolResult(HostToolResult {
                    call_id,
                    outcome: Some(outcome),
                })),
            })
            .await
            .map_err(|_| tonic::Status::unavailable("agent request stream closed"))
    }
}
