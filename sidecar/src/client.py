import argparse
import asyncio

import grpc

import chamber_agent_pb2 as messages
import chamber_agent_pb2_grpc as services


async def chat(port: int, prompt: str) -> None:
    async with grpc.aio.insecure_channel(f"127.0.0.1:{port}") as channel:
        client = services.AgentRuntimeStub(channel)
        events = client.Chat(messages.ChatRequest(session_id="demo", prompt=prompt))

        async for event in events:
            match event.WhichOneof("payload"):
                case "started":
                    print("[started]")
                case "text_delta":
                    print(event.text_delta.text, end="", flush=True)
                case "completed":
                    print("\n[completed]")
                case "error":
                    raise RuntimeError(
                        f"{event.error.code}: {event.error.message}"
                    )
                case None:
                    raise RuntimeError("Received an agent event without a payload")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("prompt")
    parser.add_argument("--port", type=int, default=50051)
    args = parser.parse_args()
    asyncio.run(chat(args.port, args.prompt))


if __name__ == "__main__":
    main()
