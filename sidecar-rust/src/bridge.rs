use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::proto;

#[derive(Clone, Default)]
pub struct ApprovalState {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<(), String>>>>>,
}

impl ApprovalState {
    pub async fn handle_decision(&self, decision: proto::ApprovalDecision) {
        let tx = {
            let mut lock = self.pending.lock().await;
            lock.remove(&decision.tool_call_id)
        };

        if let Some(tx) = tx {
            let result = match decision.outcome {
                Some(proto::approval_decision::Outcome::Approved(_)) => Ok(()),
                Some(proto::approval_decision::Outcome::Denied(denied)) => Err(denied.reason),
                None => Err("No decision provided".to_string()),
            };
            let _ = tx.send(result);
        }
    }

    pub async fn cancel_all(&self) {
        let mut lock = self.pending.lock().await;
        lock.clear();
    }
}

pub struct ApprovalBridge {
    session_id: String,
    events_tx: mpsc::Sender<Result<proto::AgentEvent, tonic::Status>>,
    state: ApprovalState,
}

impl ApprovalBridge {
    pub fn new(
        session_id: String,
        events_tx: mpsc::Sender<Result<proto::AgentEvent, tonic::Status>>,
        state: ApprovalState,
    ) -> Self {
        Self {
            session_id,
            events_tx,
            state,
        }
    }

    pub async fn emit(&self, payload: proto::agent_event::Payload) -> Result<(), ()> {
        let event = proto::AgentEvent {
            session_id: self.session_id.clone(),
            payload: Some(payload),
        };
        self.events_tx.send(Ok(event)).await.map_err(|_| ())
    }

    pub async fn request_approval(
        &self,
        tool_call_id: String,
        tool_name: String,
        arguments_json: String,
    ) -> Result<Result<(), String>, ()> {
        let (tx, rx) = oneshot::channel();
        {
            let mut lock = self.state.pending.lock().await;
            lock.insert(tool_call_id.clone(), tx);
        }

        let emit_result = self
            .emit(proto::agent_event::Payload::ApprovalRequest(
                proto::ApprovalRequest {
                    tool_call_id: tool_call_id.clone(),
                    tool_name,
                    arguments_json,
                },
            ))
            .await;

        if emit_result.is_err() {
            let mut lock = self.state.pending.lock().await;
            lock.remove(&tool_call_id);
            return Err(());
        }

        match rx.await {
            Ok(decision) => Ok(decision),
            Err(_) => Err(()),
        }
    }
}

