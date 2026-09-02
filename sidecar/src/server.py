import argparse
import asyncio
import hmac
import sys
from collections.abc import AsyncIterator

import grpc
from pydantic_ai.exceptions import AgentRunError, ModelHTTPError

import chamber_agent_pb2 as messages
import chamber_agent_pb2_grpc as services
from agent import build_agent, stream_response


class AgentRuntime(services.AgentRuntimeServicer):
    def __init__(self, auth_token: str) -> None:
        self.agent = build_agent()
        self.auth_token = auth_token

    async def Chat(
        self,
        request: messages.ChatRequest,
        context: grpc.aio.ServicerContext,
    ) -> AsyncIterator[messages.AgentEvent]:
        metadata = dict(context.invocation_metadata())
        if not hmac.compare_digest(
            metadata.get("x-chamber-token", ""),
            self.auth_token,
        ):
            await context.abort(
                grpc.StatusCode.UNAUTHENTICATED,
                "Invalid sidecar authentication token",
            )

        yield messages.AgentEvent(
            session_id=request.session_id,
            started=messages.Started(),
        )

        try:
            async for text in stream_response(self.agent, request.prompt):
                yield messages.AgentEvent(
                    session_id=request.session_id,
                    text_delta=messages.TextDelta(text=text),
                )
        except AgentRunError as error:
            yield messages.AgentEvent(
                session_id=request.session_id,
                error=messages.RuntimeError(
                    code=type(error).__name__,
                    message=str(error),
                    retryable=isinstance(error, ModelHTTPError)
                    and error.status_code >= 500,
                ),
            )
            return

        yield messages.AgentEvent(
            session_id=request.session_id,
            completed=messages.Completed(),
        )


async def serve(port: int, shutdown_on_stdin: bool) -> None:
    auth_line = (await asyncio.to_thread(sys.stdin.buffer.readline)).decode().strip()
    if not auth_line.startswith("AUTH "):
        raise RuntimeError("Expected authentication token on stdin")
    auth_token = auth_line.removeprefix("AUTH ")
    try:
        token_bytes = bytes.fromhex(auth_token)
    except ValueError as error:
        raise RuntimeError("Invalid authentication token") from error
    if len(token_bytes) != 32:
        raise RuntimeError("Invalid authentication token")

    server = grpc.aio.server()
    services.add_AgentRuntimeServicer_to_server(AgentRuntime(auth_token), server)
    bound_port = server.add_insecure_port(f"127.0.0.1:{port}")
    if bound_port == 0:
        raise RuntimeError(f"Could not bind gRPC server to port {port}")
    await server.start()
    print(f"READY {bound_port}", flush=True)
    if shutdown_on_stdin:
        command = (await asyncio.to_thread(sys.stdin.buffer.readline)).decode().strip()
        if command == "SHUTDOWN":
            await server.stop(grace=5)
        elif not command:
            await server.stop(grace=0)
        else:
            await server.stop(grace=0)
            raise RuntimeError("Expected SHUTDOWN command on stdin")
    else:
        await server.wait_for_termination()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=50051)
    parser.add_argument("--shutdown-on-stdin", action="store_true")
    args = parser.parse_args()
    asyncio.run(serve(args.port, args.shutdown_on_stdin))


if __name__ == "__main__":
    main()
