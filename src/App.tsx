import { FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import "./styles.css";

type Message = {
  role: "user" | "assistant";
  text: string;
};

type ChatEvent =
  | { type: "started"; session_id: string }
  | { type: "text_delta"; session_id: string; text: string }
  | { type: "completed"; session_id: string }
  | {
      type: "error";
      session_id: string;
      code: string;
      message: string;
      retryable: boolean;
    };

export default function App() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [prompt, setPrompt] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    const unlisten = listen<ChatEvent>("chat-event", ({ payload }) => {
      switch (payload.type) {
        case "started":
          setMessages((current) => [
            ...current,
            { role: "assistant", text: "" },
          ]);
          break;
        case "text_delta":
          setMessages((current) =>
            current.map((message, index) =>
              index === current.length - 1
                ? { ...message, text: message.text + payload.text }
                : message,
            ),
          );
          break;
        case "completed":
          setSending(false);
          break;
        case "error":
          setError(`${payload.code}: ${payload.message}`);
          setSending(false);
          break;
      }
    });

    return () => {
      void unlisten.then((stopListening) => stopListening());
    };
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const text = prompt.trim();
    if (!text || sending) {
      return;
    }

    setMessages((current) => [...current, { role: "user", text }]);
    setPrompt("");
    setError(undefined);
    setSending(true);

    try {
      await invoke("send_message", {
        sessionId: "demo",
        prompt: text,
      });
    } catch (reason) {
      setError(String(reason));
      setSending(false);
    }
  }

  return (
    <main className="app">
      <header>
        <h1>Chamber</h1>
        <p>PydanticAI via a Rust gRPC sidecar</p>
      </header>

      <section className="transcript" aria-live="polite">
        {messages.length === 0 ? (
          <p className="empty">Ask GLM something.</p>
        ) : (
          messages.map((message, index) => (
            <article className={message.role} key={index}>
              <strong>{message.role === "user" ? "You" : "Chamber"}</strong>
              <p>{message.text || "..."}</p>
            </article>
          ))
        )}
      </section>

      {error && <p className="error">{error}</p>}

      <form onSubmit={submit}>
        <input
          aria-label="Message"
          disabled={sending}
          onChange={(event) => setPrompt(event.target.value)}
          placeholder="Message Chamber"
          value={prompt}
        />
        <button disabled={sending || !prompt.trim()} type="submit">
          {sending ? "Thinking..." : "Send"}
        </button>
      </form>
    </main>
  );
}