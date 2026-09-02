mod agent_runtime;

use agent_runtime::{AgentRuntime, ChatEventPayload};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with exactly: Hello from Rust".to_owned());
    let mut runtime = AgentRuntime::connect("http://127.0.0.1:50051").await?;
    let mut events = runtime.chat("demo", prompt).await?;

    while let Some(event) = events.message().await? {
        match event.payload {
            ChatEventPayload::Started => println!("[started]"),
            ChatEventPayload::TextDelta(text) => print!("{text}"),
            ChatEventPayload::Completed => println!("\n[completed]"),
            ChatEventPayload::RuntimeError { code, message, .. } => {
                return Err(std::io::Error::other(format!("{code}: {message}")).into());
            }
        }
    }

    Ok(())
}
