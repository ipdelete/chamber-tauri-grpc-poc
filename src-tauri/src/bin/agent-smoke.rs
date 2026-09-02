use std::path::{Path, PathBuf};

use chamber_tauri_host::agent_runtime::{AgentRuntime, ChatEventPayload, generate_auth_token};
use chamber_tauri_host::lens::validate;
use chamber_tauri_host::sidecar::SidecarProcess;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with exactly: concurrent sidecars work".to_owned());
    let mind_a = scratch_mind()?;
    let mind_b = scratch_mind()?;
    let (sidecar_a, sidecar_b) = tokio::try_join!(
        SidecarProcess::start(&mind_a),
        SidecarProcess::start(&mind_b)
    )?;
    reject_invalid_token(sidecar_a.endpoint()).await?;
    let (mut runtime_a, mut runtime_b) = tokio::try_join!(
        AgentRuntime::connect(sidecar_a.endpoint(), sidecar_a.auth_token()),
        AgentRuntime::connect(sidecar_b.endpoint(), sidecar_b.auth_token())
    )?;
    let (response_a, response_b) = tokio::try_join!(
        chat(&mut runtime_a, "sidecar-a", prompt.clone()),
        chat(&mut runtime_b, "sidecar-b", prompt)
    )?;

    println!("[sidecar-a] {response_a}");
    println!("[sidecar-b] {response_b}");

    sidecar_a.kill().await?;
    println!("[sidecar-a] killed");

    cancel_after_first_delta(&mut runtime_b).await?;
    println!("[sidecar-b] cancelled active stream");

    let survivor = chat(
        &mut runtime_b,
        "sidecar-b-survivor",
        "Reply with exactly: sidecar B survived",
    )
    .await?;
    println!("[sidecar-b] {survivor}");

    prove_lens_approved(&mut runtime_b, &mind_b).await?;
    prove_lens_denied(&mut runtime_b, &mind_b).await?;

    drop(runtime_a);
    drop(runtime_b);
    sidecar_b.shutdown().await?;
    std::fs::remove_dir_all(mind_a)?;
    std::fs::remove_dir_all(mind_b)?;
    Ok(())
}

fn scratch_mind() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(std::env::temp_dir().join(format!("chamber-smoke-mind-{}", generate_auth_token()?)))
}

async fn chat(
    runtime: &mut AgentRuntime,
    session_id: &str,
    prompt: impl Into<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut events = runtime.interactive_chat(session_id, prompt).await?;
    let mut response = String::new();
    let mut completed = false;
    while let Some(event) = events.message().await? {
        match event.payload {
            ChatEventPayload::Started => {}
            ChatEventPayload::TextDelta(text) => response.push_str(&text),
            ChatEventPayload::Completed => completed = true,
            ChatEventPayload::RuntimeError { code, message, .. } => {
                return Err(std::io::Error::other(format!("{code}: {message}")).into());
            }
            ChatEventPayload::ApprovalRequest { tool_name, .. } => {
                return Err(std::io::Error::other(format!(
                    "unexpected approval request: {tool_name}"
                ))
                .into());
            }
            ChatEventPayload::LensChanged(lens) => {
                return Err(
                    std::io::Error::other(format!("unexpected lens change: {}", lens.id)).into(),
                );
            }
        }
    }

    if !completed {
        return Err(std::io::Error::other("chat ended without a completed event").into());
    }

    Ok(response)
}

async fn reject_invalid_token(endpoint: String) -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = AgentRuntime::connect(endpoint, "invalid-token").await?;
    // The abort can surface either on the initial response or on the first message.
    let outcome = match runtime
        .interactive_chat("unauthenticated", "This must not run")
        .await
    {
        Err(status) => Err(status),
        Ok(mut events) => events.message().await,
    };
    match outcome {
        Err(status) if status.code() == tonic::Code::Unauthenticated => {
            println!("[authentication] invalid token rejected");
            Ok(())
        }
        Err(status) => Err(std::io::Error::other(format!(
            "expected unauthenticated, received {status}"
        ))
        .into()),
        Ok(_) => Err(std::io::Error::other("invalid token was accepted").into()),
    }
}

async fn cancel_after_first_delta(
    runtime: &mut AgentRuntime,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut events = runtime
        .interactive_chat(
            "sidecar-b-cancelled",
            "Write a detailed 500-word explanation of process isolation.",
        )
        .await?;

    while let Some(event) = events.message().await? {
        match event.payload {
            ChatEventPayload::TextDelta(_) => return Ok(()),
            ChatEventPayload::Completed => {
                return Err(std::io::Error::other("chat completed before cancellation").into());
            }
            ChatEventPayload::RuntimeError { code, message, .. } => {
                return Err(std::io::Error::other(format!("{code}: {message}")).into());
            }
            _ => {}
        }
    }

    Err(std::io::Error::other("chat ended before cancellation").into())
}

const LENS_PROMPT: &str =
    "Create a Canvas Lens named Smoke Board with one card saying Lens works. You must call lens_upsert.";

async fn prove_lens_approved(
    runtime: &mut AgentRuntime,
    mind_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut events = runtime.interactive_chat("lens-approved", LENS_PROMPT).await?;
    let mut approved = false;
    let mut changed = None;

    while let Some(event) = events.message().await? {
        match event.payload {
            ChatEventPayload::ApprovalRequest {
                tool_call_id,
                tool_name,
                ..
            } => {
                if tool_name != "lens_upsert" {
                    return Err(std::io::Error::other(format!(
                        "unexpected approval request: {tool_name}"
                    ))
                    .into());
                }
                approved = true;
                events.send_approval_decision(tool_call_id, Ok(())).await?;
            }
            ChatEventPayload::LensChanged(lens) => {
                validate(&lens)?;
                changed = Some(lens);
            }
            ChatEventPayload::Completed => break,
            ChatEventPayload::RuntimeError { code, message, .. } => {
                return Err(std::io::Error::other(format!("{code}: {message}")).into());
            }
            _ => {}
        }
    }

    if !approved {
        return Err(std::io::Error::other("agent did not request approval").into());
    }
    let lens = changed.ok_or_else(|| std::io::Error::other("no lens change was announced"))?;
    let directory = mind_root.join(".github").join("lens").join(&lens.id);
    if std::fs::read_to_string(directory.join("index.html"))? != lens.html {
        return Err(std::io::Error::other("lens HTML on disk does not match the event").into());
    }
    if !directory.join("view.json").is_file() {
        return Err(std::io::Error::other("lens manifest was not written").into());
    }
    println!("[approval] lens_upsert approved and written to {}", lens.id);
    Ok(())
}

async fn prove_lens_denied(
    runtime: &mut AgentRuntime,
    mind_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let before = lens_count(mind_root);
    let mut events = runtime.interactive_chat("lens-denied", LENS_PROMPT).await?;
    let mut denied = false;
    let mut completed = false;

    while let Some(event) = events.message().await? {
        match event.payload {
            ChatEventPayload::ApprovalRequest { tool_call_id, .. } => {
                denied = true;
                events
                    .send_approval_decision(
                        tool_call_id,
                        Err("The user declined this lens.".to_owned()),
                    )
                    .await?;
            }
            ChatEventPayload::LensChanged(lens) => {
                return Err(std::io::Error::other(format!(
                    "denied tool still changed lens {}",
                    lens.id
                ))
                .into());
            }
            ChatEventPayload::Completed => {
                completed = true;
                break;
            }
            ChatEventPayload::RuntimeError { code, message, .. } => {
                return Err(std::io::Error::other(format!("{code}: {message}")).into());
            }
            _ => {}
        }
    }

    if !denied {
        return Err(std::io::Error::other("agent did not request approval").into());
    }
    if !completed {
        return Err(std::io::Error::other("denied run did not complete").into());
    }
    if lens_count(mind_root) != before {
        return Err(std::io::Error::other("denied run wrote a lens").into());
    }
    println!("[approval] lens_upsert denied and the run still completed");
    Ok(())
}

fn lens_count(mind_root: &Path) -> usize {
    std::fs::read_dir(mind_root.join(".github").join("lens"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}
