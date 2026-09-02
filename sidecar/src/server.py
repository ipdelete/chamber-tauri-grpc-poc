import argparse
import asyncio
import hmac
import sys
from collections.abc import AsyncIterator
from pathlib import Path

import grpc
from pydantic_ai.exceptions import AgentRunError, ModelHTTPError

import chamber_agent_pb2 as messages
import chamber_agent_pb2_grpc as services
from agent import build_agent, stream_response


class AgentRuntime(services.AgentRuntimeServicer):
    def __init__(self, auth_token: str, mind_root: Path) -> None:
        self.auth_token = auth_token
        self.mind_root = mind_root

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

        bridge = ApprovalBridge(first.session_id)
        reader = asyncio.create_task(bridge.receive_decisions(request_iterator))
        reader.add_done_callback(bridge.reader_finished)
        runner = asyncio.create_task(
            self._run_interactive_agent(bridge, first.prompt.text)
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
        bridge: "ApprovalBridge",
        prompt: str,
    ) -> None:
        completed = False
        try:
            await bridge.emit(started=messages.Started())
            agent = build_agent(
                self.mind_root,
                bridge.request_approval,
                bridge.announce_lens,
            )
            async for text in stream_response(agent, prompt):
                await bridge.emit(text_delta=messages.TextDelta(text=text))
            completed = True
        except asyncio.CancelledError:
            raise
        except BaseException as error:  # noqa: BLE001 - the stream must always terminate
            await bridge.emit(
                error=messages.RuntimeError(
                    code=type(error).__name__,
                    message=str(error)
                    if isinstance(error, AgentRunError)
                    else "The agent run failed",
                    retryable=isinstance(error, ModelHTTPError)
                    and error.status_code >= 500,
                )
            )
        finally:
            if completed:
                await bridge.emit(completed=messages.Completed())
            await bridge.events.put(None)


class ApprovalBridge:
    """Carries approval requests down to the host and decisions back up.

    Correlation uses PydanticAI's own tool_call_id, so the sidecar mints no IDs.
    """

    def __init__(self, session_id: str) -> None:
        self.session_id = session_id
        self.events: asyncio.Queue[messages.AgentEvent | None] = asyncio.Queue()
        self.pending: dict[str, asyncio.Future[str | None]] = {}
        self.closed: str | None = None

    async def emit(self, **payload: object) -> None:
        await self.events.put(
            messages.AgentEvent(session_id=self.session_id, **payload)
        )

    async def announce_lens(self, lens: dict[str, str]) -> None:
        await self.emit(lens_changed=messages.LensChanged(**lens))

    async def request_approval(
        self,
        tool_call_id: str,
        tool_name: str,
        arguments_json: str,
    ) -> str | None:
        """Return None when approved, or the denial reason."""
        if self.closed is not None:
            raise RuntimeError(self.closed)
        if tool_call_id in self.pending:
            raise RuntimeError(f"Duplicate approval request for {tool_call_id!r}")

        decision = asyncio.get_running_loop().create_future()
        self.pending[tool_call_id] = decision
        await self.emit(
            approval_request=messages.ApprovalRequest(
                tool_call_id=tool_call_id,
                tool_name=tool_name,
                arguments_json=arguments_json,
            )
        )
        try:
            return await decision
        finally:
            self.pending.pop(tool_call_id, None)

    async def receive_decisions(
        self,
        requests: AsyncIterator[messages.HostMessage],
    ) -> None:
        async for request in requests:
            if request.session_id != self.session_id:
                self.close("Approval decision used the wrong session")
                return
            if request.WhichOneof("payload") != "approval_decision":
                self.close("Expected an approval decision")
                return
            decision = request.approval_decision
            pending = self.pending.get(decision.tool_call_id)
            if pending is None:
                self.close("Host answered an unknown tool call ID")
                return
            if pending.done():
                self.close("Host sent a duplicate approval decision")
                return
            outcome = decision.WhichOneof("outcome")
            if outcome == "approved":
                pending.set_result(None)
            elif outcome == "denied":
                pending.set_result(decision.denied.reason or "The tool call was denied.")
            else:
                self.close("Approval decision omitted its outcome")
                return

        self.close("Host connection closed")

    def reader_finished(self, task: "asyncio.Task[None]") -> None:
        """Close the bridge if the reader stopped without saying why."""
        if task.cancelled():
            return
        error = task.exception()
        if error is not None:
            self.close(f"Host connection failed: {error}")
        elif self.closed is None:
            self.close("Host connection closed")

    def close(self, message: str) -> None:
        """Fail every waiting approval and refuse any later one."""
        self.closed = message
        for pending in self.pending.values():
            if not pending.done():
                pending.set_exception(RuntimeError(message))


async def serve(port: int, shutdown_on_stdin: bool, mind_root: Path) -> None:
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
    services.add_AgentRuntimeServicer_to_server(
        AgentRuntime(auth_token, mind_root), server
    )
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
    parser.add_argument("--mind-root", type=Path, required=True)
    args = parser.parse_args()
    asyncio.run(serve(args.port, args.shutdown_on_stdin, args.mind_root))


if __name__ == "__main__":
    main()
