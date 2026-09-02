import {
  FormEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import "./styles.css";

type Message = {
  role: "user" | "assistant";
  text: string;
};

type Lens = {
  id: string;
  name: string;
  icon: string;
  html: string;
};

type ChatEvent =
  | { type: "started"; session_id: string }
  | { type: "text_delta"; session_id: string; text: string }
  | { type: "completed"; session_id: string }
  | { type: "cancelled"; session_id: string }
  | {
      type: "error";
      session_id: string;
      code: string;
      message: string;
      retryable: boolean;
    };

const canvasBridge = `
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:">
<style>
  :root { color-scheme: dark; --ch-background:#0f141b; --ch-card:#151c25; --ch-foreground:#e6edf3; --ch-muted:#9aa7b4; --ch-border:#30363d; --ch-genesis:#7c5cff; font-family:Inter,system-ui,sans-serif; }
  * { box-sizing:border-box; }
  body,.ch-page { margin:0; min-height:100vh; padding:24px; color:var(--ch-foreground); background:var(--ch-background); }
  .ch-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:14px; }
  .ch-card { padding:16px; border:1px solid var(--ch-border); border-radius:12px; background:var(--ch-card); }
  .ch-button,.ch-button-secondary { padding:9px 12px; border:0; border-radius:8px; color:white; background:var(--ch-genesis); cursor:pointer; }
  .ch-button-secondary { border:1px solid var(--ch-border); background:transparent; }
  .ch-input { padding:9px; border:1px solid var(--ch-border); border-radius:8px; color:var(--ch-foreground); background:var(--ch-background); }
  .ch-table { width:100%; border-collapse:collapse; }
  .ch-table th,.ch-table td { padding:10px; border-bottom:1px solid var(--ch-border); text-align:left; }
  .ch-badge { display:inline-block; padding:3px 7px; border-radius:999px; color:#d8ceff; background:#352b5c; font-size:12px; }
  .ch-muted { color:var(--ch-muted); }
</style>
<script>
  window.canvas = {
    sendAction(action, data = {}) {
      parent.postMessage({ source: "chamber-canvas", action, data }, "*");
      return Promise.resolve(new Response(JSON.stringify({ ok: true }), { headers: { "content-type": "application/json" } }));
    }
  };
<\/script>`;

function withCanvasBridge(html: string) {
  return /<head(?:\s[^>]*)?>/i.test(html)
    ? html.replace(/<head(?:\s[^>]*)?>/i, (head) => `${head}${canvasBridge}`)
    : `${canvasBridge}${html}`;
}

export default function App() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [prompt, setPrompt] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string>();
  const [lens, setLens] = useState<Lens>();
  const [activeView, setActiveView] = useState<"chat" | "lens">("chat");
  const frame = useRef<HTMLIFrameElement>(null);

  useEffect(() => {
    const chatEvents = listen<ChatEvent>("chat-event", ({ payload }) => {
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
        case "cancelled":
          setMessages((current) =>
            current.map((message, index) =>
              index === current.length - 1
                ? {
                    ...message,
                    text: message.text
                      ? `${message.text}\n\nStopped.`
                      : "Stopped.",
                  }
                : message,
            ),
          );
          setSending(false);
          break;
        case "error":
          setError(`${payload.code}: ${payload.message}`);
          setSending(false);
          break;
      }
    });
    const lensEvents = listen<Lens>("lens-event", ({ payload }) => {
      setLens(payload);
      setActiveView("lens");
    });

    return () => {
      void chatEvents.then((stop) => stop());
      void lensEvents.then((stop) => stop());
    };
  }, []);

  const sendPrompt = useCallback(
    async (text: string, displayText = text) => {
      const trimmed = text.trim();
      if (!trimmed || sending) {
        return;
      }

      setMessages((current) => [
        ...current,
        { role: "user", text: displayText },
      ]);
      setPrompt("");
      setError(undefined);
      setSending(true);

      try {
        await invoke("send_message", {
          sessionId: "demo",
          prompt: trimmed,
        });
      } catch (reason) {
        setError(String(reason));
        setSending(false);
      }
    },
    [sending],
  );

  useEffect(() => {
    function receiveCanvasAction(event: MessageEvent) {
      if (
        event.source !== frame.current?.contentWindow ||
        event.data?.source !== "chamber-canvas" ||
        typeof event.data.action !== "string" ||
        !lens
      ) {
        return;
      }

      const action = JSON.stringify({
        action: event.data.action,
        data: event.data.data ?? {},
      });
      void sendPrompt(
        lensUpdatePrompt(
          lens,
          `The user interacted with the Canvas: ${action}`,
        ),
        `Canvas action: ${event.data.action}`,
      );
    }

    window.addEventListener("message", receiveCanvasAction);
    return () => window.removeEventListener("message", receiveCanvasAction);
  }, [lens, sendPrompt]);

  function submit(event: FormEvent) {
    event.preventDefault();
    const request =
      activeView === "lens" && lens
        ? lensUpdatePrompt(lens, prompt)
        : prompt;
    void sendPrompt(request, prompt);
  }

  async function cancel() {
    try {
      await invoke("cancel_message", { sessionId: "demo" });
    } catch (reason) {
      setError(String(reason));
    }
  }

  return (
    <main className="shell">
      <aside className="sidebar">
        <h1>Chamber</h1>
        <button
          className={activeView === "chat" ? "nav active" : "nav"}
          onClick={() => setActiveView("chat")}
          type="button"
        >
          <span>✦</span> Chat
        </button>
        <p className="nav-label">Lens</p>
        {lens ? (
          <button
            className={activeView === "lens" ? "nav active" : "nav"}
            onClick={() => setActiveView("lens")}
            type="button"
          >
            <span>▦</span> {lens.name}
          </button>
        ) : (
          <p className="no-lens">Ask the agent to build a view.</p>
        )}
        <p className="runtime">PydanticAI sidecar connected</p>
      </aside>

      <section className="workspace">
        <header>
          <div>
            <h2>{activeView === "lens" && lens ? lens.name : "Chat"}</h2>
            <p>
              {activeView === "lens"
                ? "Sandboxed Canvas Lens"
                : "PydanticAI via a Rust gRPC sidecar"}
            </p>
          </div>
          {activeView === "lens" && (
            <span className={sending ? "lens-status working" : "lens-status"}>
              {sending ? "Agent updating..." : "Live"}
            </span>
          )}
        </header>

        {activeView === "chat" ? (
          <section className="chat">
            <div className="transcript" aria-live="polite">
              {messages.length === 0 ? (
                <p className="empty">
                  Try: "Build a release dashboard as a Canvas Lens."
                </p>
              ) : (
                messages.map((message, index) => (
                  <article className={message.role} key={index}>
                    <strong>
                      {message.role === "user" ? "You" : "Chamber"}
                    </strong>
                    <p>{message.text || "..."}</p>
                  </article>
                ))
              )}
            </div>
          </section>
        ) : (
          <section className="lens">
            <iframe
              ref={frame}
              sandbox="allow-scripts"
              srcDoc={lens ? withCanvasBridge(lens.html) : ""}
              title={lens?.name}
            />
            {sending && (
              <div className="lens-overlay">The agent is updating this Lens...</div>
            )}
          </section>
        )}

        {error && <p className="error">{error}</p>}

        <form onSubmit={submit}>
          <input
            aria-label="Message"
            disabled={sending}
            onChange={(event) => setPrompt(event.target.value)}
            placeholder={
              activeView === "lens"
                ? "Ask the agent to change this Lens"
                : "Message Chamber"
            }
            value={prompt}
          />
          {sending ? (
            <button className="stop" onClick={cancel} type="button">
              Stop
            </button>
          ) : (
            <button disabled={!prompt.trim()} type="submit">
              Send
            </button>
          )}
        </form>
      </section>
    </main>
  );
}

function lensUpdatePrompt(lens: Lens, request: string) {
  return (
    `The user wants to change the current Canvas Lens "${lens.name}". ` +
    `Keep the same id "${lens.id}" and call lens_upsert with the revised UI.\n\n` +
    `Request: ${request}\n\nCurrent HTML:\n${lens.html}`
  );
}
