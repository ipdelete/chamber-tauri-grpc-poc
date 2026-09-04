package server

import (
	"context"
	"crypto/subtle"
	"errors"
	"io"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"

	"github.com/microsoft/agent-framework-go/agent"
	"github.com/microsoft/agent-framework-go/message"

	agentpkg "sidecar-go/internal/agent"
	"sidecar-go/internal/lens"
	"sidecar-go/proto/agentpb"
)

type RuntimeServer struct {
	agentpb.UnimplementedAgentRuntimeServer
	authToken string
	mindRoot  string
}

func NewRuntimeServer(authToken, mindRoot string) *RuntimeServer {
	return &RuntimeServer{
		authToken: authToken,
		mindRoot:  mindRoot,
	}
}

func (s *RuntimeServer) Interact(stream agentpb.AgentRuntime_InteractServer) error {
	if err := s.authenticate(stream.Context()); err != nil {
		return err
	}

	first, err := stream.Recv()
	if err != nil {
		if errors.Is(err, io.EOF) {
			return status.Error(codes.InvalidArgument, "Interact requires an initial prompt")
		}
		return err
	}

	if first.GetPrompt() == nil {
		return status.Error(codes.InvalidArgument, "First Interact message must be a prompt")
	}

	sessionID := first.GetSessionId()
	promptText := first.GetPrompt().GetText()

	bridge := NewApprovalBridge(sessionID)
	ctx, cancel := context.WithCancel(stream.Context())
	defer cancel()

	// Reader goroutine
	go func() {
		for {
			msg, err := stream.Recv()
			if err != nil {
				bridge.Close(err)
				cancel()
				return
			}
			if err := bridge.ReceiveDecision(msg); err != nil {
				bridge.Close(err)
				cancel()
				return
			}
		}
	}()

	// Agent runner goroutine
	go func() {
		defer bridge.Close(nil)
		s.runInteractiveAgent(ctx, bridge, promptText)
	}()

	for event := range bridge.Events() {
		if err := stream.Send(event); err != nil {
			return err
		}
	}

	return nil
}

func (s *RuntimeServer) authenticate(ctx context.Context) error {
	md, ok := metadata.FromIncomingContext(ctx)
	if !ok {
		return status.Error(codes.Unauthenticated, "Invalid sidecar authentication token")
	}

	tokens := md.Get("x-chamber-token")
	if len(tokens) == 0 || subtle.ConstantTimeCompare([]byte(tokens[0]), []byte(s.authToken)) != 1 {
		return status.Error(codes.Unauthenticated, "Invalid sidecar authentication token")
	}

	return nil
}

func (s *RuntimeServer) runInteractiveAgent(
	ctx context.Context,
	bridge *ApprovalBridge,
	prompt string,
) {
	if err := bridge.Emit(&agentpb.AgentEvent{
		Payload: &agentpb.AgentEvent_Started{
			Started: &agentpb.Started{},
		},
	}); err != nil {
		return
	}

	onLensChanged := func(snap lens.LensSnapshot) {
		_ = bridge.Emit(&agentpb.AgentEvent{
			Payload: &agentpb.AgentEvent_LensChanged{
				LensChanged: &agentpb.LensChanged{
					Id:   snap.ID,
					Name: snap.Name,
					Icon: snap.Icon,
					Html: snap.HTML,
				},
			},
		})
	}

	a, err := agentpkg.BuildAgent(s.mindRoot, onLensChanged)
	if err != nil {
		_ = bridge.Emit(&agentpb.AgentEvent{
			Payload: &agentpb.AgentEvent_Error{
				Error: &agentpb.RuntimeError{
					Code:      "AgentInitError",
					Message:   err.Error(),
					Retryable: false,
				},
			},
		})
		return
	}

	session, err := a.CreateSession(ctx)
	if err != nil {
		if ctx.Err() != nil {
			return
		}
		_ = bridge.Emit(&agentpb.AgentEvent{
			Payload: &agentpb.AgentEvent_Error{
				Error: &agentpb.RuntimeError{
					Code:      "SessionInitError",
					Message:   err.Error(),
					Retryable: false,
				},
			},
		})
		return
	}

	currentMessages := []*message.Message{message.NewText(prompt)}
	completed := false
	processedToolCallIDs := make(map[string]bool)

turnLoop:
	for {
		if ctx.Err() != nil {
			return
		}

		var approvalRequests []*message.ToolApprovalRequestContent
		seenRequests := make(map[string]bool)
		updates := a.Run(
			ctx,
			currentMessages,
			agent.WithSession(session),
			agent.Stream(true),
		)

		for update, err := range updates {
			if err != nil {
				if ctx.Err() != nil {
					return
				}
				_ = bridge.Emit(&agentpb.AgentEvent{
					Payload: &agentpb.AgentEvent_Error{
						Error: &agentpb.RuntimeError{
							Code:      "AgentRunError",
							Message:   err.Error(),
							Retryable: false,
						},
					},
				})
				return
			}

			if update == nil {
				continue
			}

			for _, c := range update.Contents {
				if tc, ok := c.(*message.TextContent); ok && tc.Text != "" {
					if err := bridge.Emit(&agentpb.AgentEvent{
						Payload: &agentpb.AgentEvent_TextDelta{
							TextDelta: &agentpb.TextDelta{Text: tc.Text},
						},
					}); err != nil {
						return
					}
				}

				if req, ok := c.(*message.ToolApprovalRequestContent); ok {
					callID := req.ToolCall.GetCallID()
					if !processedToolCallIDs[callID] && !seenRequests[req.RequestID] {
						seenRequests[req.RequestID] = true
						approvalRequests = append(approvalRequests, req)
					}
				}
			}
		}

		if len(approvalRequests) > 0 {
			type decisionResult struct {
				approved bool
				reason   string
			}
			decisions := make(map[string]decisionResult)

			var userResponses []message.Content
			for _, req := range approvalRequests {
				toolCallID := req.ToolCall.GetCallID()
				processedToolCallIDs[toolCallID] = true
				dec, exists := decisions[toolCallID]
				if !exists {
					toolName := ""
					argumentsJSON := ""
					if fcc, ok := req.ToolCall.(*message.FunctionCallContent); ok {
						toolName = fcc.Name
						argumentsJSON = fcc.Arguments
					}

					if err := bridge.Emit(&agentpb.AgentEvent{
						Payload: &agentpb.AgentEvent_ApprovalRequest{
							ApprovalRequest: &agentpb.ApprovalRequest{
								ToolCallId:    toolCallID,
								ToolName:      toolName,
								ArgumentsJson: argumentsJSON,
							},
						},
					}); err != nil {
						return
					}

					decision, err := bridge.WaitForDecision(ctx, toolCallID)
					if err != nil {
						return
					}

					if decision.GetApproved() != nil {
						dec = decisionResult{approved: true}
					} else {
						reason := "The tool call was denied."
						if denied := decision.GetDenied(); denied != nil && denied.Reason != "" {
							reason = denied.Reason
						}
						dec = decisionResult{approved: false, reason: reason}
					}
					decisions[toolCallID] = dec
				}

				resp := req.CreateResponse(dec.approved, dec.reason)
				userResponses = append(userResponses, resp)
			}

			currentMessages = []*message.Message{message.New(userResponses...)}
		} else {
			completed = true
			break turnLoop
		}
	}

	if completed {
		_ = bridge.Emit(&agentpb.AgentEvent{
			Payload: &agentpb.AgentEvent_Completed{
				Completed: &agentpb.Completed{},
			},
		})
	}
}
