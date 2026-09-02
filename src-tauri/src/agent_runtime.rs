mod protocol {
    tonic::include_proto!("chamber.agent.v1");
}

use std::fmt::Write;

use protocol::ChatRequest;
use protocol::agent_event::Payload;
use protocol::agent_runtime_client::AgentRuntimeClient;
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
        let mut request = Request::new(ChatRequest {
            session_id: session_id.into(),
            prompt: prompt.into(),
        });
        request.metadata_mut().insert(
            "x-chamber-token",
            self.auth_token
                .parse()
                .map_err(|_| tonic::Status::internal("invalid sidecar authentication token"))?,
        );
        let response = self.client.chat(request).await?;

        Ok(ChatStream {
            inner: response.into_inner(),
        })
    }
}

pub struct ChatStream {
    inner: tonic::Streaming<protocol::AgentEvent>,
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
            None => return Err(tonic::Status::data_loss("agent event has no payload")),
        };

        Ok(Some(ChatEvent {
            session_id: event.session_id,
            payload,
        }))
    }
}
