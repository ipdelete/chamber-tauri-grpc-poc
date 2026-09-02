import argparse
import asyncio
import hmac
import secrets
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
        await self._authenticate(context)

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

    async def Interact(
        self,
        request_iterator: AsyncIterator[messages.HostMessage],
        context: grpc.aio.ServicerContext,
    ) -> AsyncIterator[messages.AgentEvent]:
        await self._authenticate(context)

        try:
            first = await anext(request_iterator)
        except StopAsyncIteration:
            await context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "Interact requires an initial prompt",
            )
            return

        if first.WhichOneof("payload") != "prompt":
            await context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "First Interact message must be a prompt",
            )
            return

        bridge = HostToolBridge(first.session_id)
        reader = asyncio.create_task(bridge.receive_results(request_iterator))
        runner = asyncio.create_task(
            self._run_interactive_agent(
                bridge,
                first.prompt.text,
            )
        )

        try:
            while event := await bridge.events.get():
                yield event
        finally:
            runner.cancel()
            reader.cancel()
            await asyncio.gather(runner, reader, return_exceptions=True)

    async def _authenticate(
        self,
        context: grpc.aio.ServicerContext,
    ) -> None:
        metadata = dict(context.invocation_metadata())
        if not hmac.compare_digest(
            metadata.get("x-chamber-token", ""),
            self.auth_token,
        ):
            await context.abort(
                grpc.StatusCode.UNAUTHENTICATED,
                "Invalid sidecar authentication token",
            )

    async def _run_interactive_agent(
        self,
        bridge: "HostToolBridge",
        prompt: str,
    ) -> None:
        await bridge.emit(started=messages.Started())
        completed = False
        try:
            agent = build_agent(bridge.call)
            async for text in stream_response(agent, prompt):
                await bridge.emit(text_delta=messages.TextDelta(text=text))
            completed = True
        except AgentRunError as error:
            await bridge.emit(
                error=messages.RuntimeError(
                    code=type(error).__name__,
                    message=str(error),
                    retryable=isinstance(error, ModelHTTPError)
                    and error.status_code >= 500,
                )
            )
        finally:
            if completed:
                await bridge.emit(completed=messages.Completed())
            await bridge.events.put(None)


class HostToolBridge:
    def __init__(self, session_id: str) -> None:
        self.session_id = session_id
        self.events: asyncio.Queue[messages.AgentEvent | None] = asyncio.Queue()
        self.pending: dict[str, asyncio.Future[str]] = {}

    async def emit(self, **payload: object) -> None:
        await self.events.put(
            messages.AgentEvent(session_id=self.session_id, **payload)
        )

    async def call(self, name: str, arguments_json: str) -> str:
        call_id = secrets.token_hex(16)
        result = asyncio.get_running_loop().create_future()
        self.pending[call_id] = result
        await self.emit(
            host_tool_call=messages.HostToolCall(
                call_id=call_id,
                name=name,
                arguments_json=arguments_json,
            )
        )
        try:
            return await result
        finally:
            self.pending.pop(call_id, None)

    async def receive_results(
        self,
        requests: AsyncIterator[messages.HostMessage],
    ) -> None:
        async for request in requests:
            if request.session_id != self.session_id:
                self.fail_pending("Host tool result used the wrong session")
                return
            if request.WhichOneof("payload") != "tool_result":
                self.fail_pending("Expected a host tool result")
                return
            result = request.tool_result
            pending = self.pending.get(result.call_id)
            if pending is None:
                self.fail_pending("Host returned an unknown tool call ID")
                return
            if pending.done():
                self.fail_pending("Host returned a duplicate tool result")
                return
            if result.WhichOneof("outcome") == "result_json":
                pending.set_result(result.result_json)
            elif result.WhichOneof("outcome") == "error":
                pending.set_exception(RuntimeError(result.error))
            else:
                pending.set_exception(RuntimeError("Host tool result omitted its outcome"))

        self.fail_pending("Host connection closed")

    def fail_pending(self, message: str) -> None:
        for pending in self.pending.values():
            if not pending.done():
                pending.set_exception(RuntimeError(message))


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
