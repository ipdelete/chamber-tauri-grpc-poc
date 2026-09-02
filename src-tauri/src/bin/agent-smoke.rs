use chamber_tauri_host::agent_runtime::{AgentRuntime, ChatEventPayload, generate_auth_token};
use chamber_tauri_host::lens::upsert;
use chamber_tauri_host::sidecar::SidecarProcess;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with exactly: concurrent sidecars work".to_owned());
    let (sidecar_a, sidecar_b) =
        tokio::try_join!(SidecarProcess::start(), SidecarProcess::start())?;
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

    prove_lens_host_tool(&mut runtime_b).await?;

    drop(runtime_a);
    drop(runtime_b);
    sidecar_b.shutdown().await?;
    Ok(())
}

async fn chat(
    runtime: &mut AgentRuntime,
    session_id: &str,
    prompt: impl Into<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut events = runtime.chat(session_id, prompt).await?;
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
            ChatEventPayload::HostToolCall { name, .. } => {
                return Err(
                    std::io::Error::other(format!("unexpected host tool call: {name}")).into(),
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
    match runtime.chat("unauthenticated", "This must not run").await {
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
        .chat(
            "sidecar-b-cancelled",
            "Write a detailed 500-word explanation of process isolation.",
        )
        .await?;

    while let Some(event) = events.message().await? {
        match event.payload {
            ChatEventPayload::Started => {}
            ChatEventPayload::TextDelta(_) => return Ok(()),
            ChatEventPayload::Completed => {
                return Err(std::io::Error::other("chat completed before cancellation").into());
            }
            ChatEventPayload::RuntimeError { code, message, .. } => {
                return Err(std::io::Error::other(format!("{code}: {message}")).into());
            }
            ChatEventPayload::HostToolCall { name, .. } => {
                return Err(
                    std::io::Error::other(format!("unexpected host tool call: {name}")).into(),
                );
            }
        }
    }

    Err(std::io::Error::other("chat ended before cancellation").into())
}

async fn prove_lens_host_tool(
    runtime: &mut AgentRuntime,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("chamber-lens-smoke-{}", generate_auth_token()?));
    let mut events = runtime
        .interactive_chat(
            "lens-tool",
            "Create a Canvas Lens named Smoke Board with one card saying Lens works. You must call lens_upsert.",
        )
        .await?;
    let mut called = false;

    while let Some(event) = events.message().await? {
        match event.payload {
            ChatEventPayload::Started | ChatEventPayload::TextDelta(_) => {}
            ChatEventPayload::HostToolCall {
                call_id,
                name,
                arguments_json,
            } => {
                if name != "lens_upsert" {
                    return Err(std::io::Error::other(format!(
                        "unexpected host tool call: {name}"
                    ))
                    .into());
                }
                let result = upsert(&root, &arguments_json).map(|lens| {
                    called = true;
                    serde_json::json!({
                        "ok": true,
                        "id": lens.id,
                        "message": "Lens saved and displayed"
                    })
                    .to_string()
                });
                events.send_tool_result(call_id, result).await?;
            }
            ChatEventPayload::Completed => break,
            ChatEventPayload::RuntimeError { code, message, .. } => {
                return Err(std::io::Error::other(format!("{code}: {message}")).into());
            }
        }
    }

    if !called {
        return Err(std::io::Error::other("agent did not call lens_upsert").into());
    }
    std::fs::remove_dir_all(root)?;
    println!("[host-tool] lens_upsert completed");
    Ok(())
}
