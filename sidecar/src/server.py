import argparse
import asyncio
from collections.abc import AsyncIterator

import grpc
from pydantic_ai.exceptions import AgentRunError, ModelHTTPError

import chamber_agent_pb2 as messages
import chamber_agent_pb2_grpc as services
from agent import build_agent, stream_response


class AgentRuntime(services.AgentRuntimeServicer):
    def __init__(self) -> None:
        self.agent = build_agent()

    async def Chat(
        self,
        request: messages.ChatRequest,
        _context: grpc.aio.ServicerContext,
    ) -> AsyncIterator[messages.AgentEvent]:
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


async def serve(port: int) -> None:
    server = grpc.aio.server()
    services.add_AgentRuntimeServicer_to_server(AgentRuntime(), server)
    bound_port = server.add_insecure_port(f"127.0.0.1:{port}")
    if bound_port == 0:
        raise RuntimeError(f"Could not bind gRPC server to port {port}")
    await server.start()
    print(f"READY {bound_port}", flush=True)
    await server.wait_for_termination()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=50051)
    args = parser.parse_args()
    asyncio.run(serve(args.port))


if __name__ == "__main__":
    main()
