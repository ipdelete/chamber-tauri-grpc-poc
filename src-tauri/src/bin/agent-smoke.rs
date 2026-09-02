use chamber_tauri_host::agent_runtime::{AgentRuntime, ChatEventPayload};
use chamber_tauri_host::sidecar::SidecarProcess;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with exactly: concurrent sidecars work".to_owned());
    let (sidecar_a, sidecar_b) =
        tokio::try_join!(SidecarProcess::start(), SidecarProcess::start())?;
    let (mut runtime_a, mut runtime_b) = tokio::try_join!(
        AgentRuntime::connect(sidecar_a.endpoint()),
        AgentRuntime::connect(sidecar_b.endpoint())
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
        }
    }

    if !completed {
        return Err(std::io::Error::other("chat ended without a completed event").into());
    }

    Ok(response)
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
        }
    }

    Err(std::io::Error::other("chat ended before cancellation").into())
}
