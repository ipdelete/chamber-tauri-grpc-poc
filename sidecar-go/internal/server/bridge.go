package server

import (
	"context"
	"errors"
	"fmt"
	"sync"

	"sidecar-go/proto/agentpb"
)

type ApprovalBridge struct {
	sessionID string
	events    chan *agentpb.AgentEvent
	mu        sync.Mutex
	pending   map[string]chan *agentpb.ApprovalDecision
	closed    bool
	closeErr  error
}

func NewApprovalBridge(sessionID string) *ApprovalBridge {
	return &ApprovalBridge{
		sessionID: sessionID,
		events:    make(chan *agentpb.AgentEvent, 64),
		pending:   make(map[string]chan *agentpb.ApprovalDecision),
	}
}

func (b *ApprovalBridge) Events() <-chan *agentpb.AgentEvent {
	return b.events
}

func (b *ApprovalBridge) Emit(event *agentpb.AgentEvent) error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return errors.New("bridge closed")
	}
	event.SessionId = b.sessionID
	b.events <- event
	return nil
}

func (b *ApprovalBridge) WaitForDecision(ctx context.Context, toolCallID string) (*agentpb.ApprovalDecision, error) {
	ch := make(chan *agentpb.ApprovalDecision, 1)

	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		return nil, fmt.Errorf("bridge is closed: %w", b.closeErr)
	}
	if _, exists := b.pending[toolCallID]; exists {
		b.mu.Unlock()
		return nil, fmt.Errorf("duplicate approval request for %s", toolCallID)
	}
	b.pending[toolCallID] = ch
	b.mu.Unlock()

	defer func() {
		b.mu.Lock()
		delete(b.pending, toolCallID)
		b.mu.Unlock()
	}()

	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case dec, ok := <-ch:
		if !ok {
			return nil, errors.New("bridge closed while waiting for decision")
		}
		return dec, nil
	}
}

func (b *ApprovalBridge) ReceiveDecision(msg *agentpb.HostMessage) error {
	if msg.GetSessionId() != b.sessionID {
		return errors.New("approval decision used the wrong session")
	}
	dec := msg.GetApprovalDecision()
	if dec == nil {
		return errors.New("expected an approval decision")
	}

	b.mu.Lock()
	defer b.mu.Unlock()
	ch, ok := b.pending[dec.GetToolCallId()]
	if !ok {
		return fmt.Errorf("host answered an unknown tool call ID: %s", dec.GetToolCallId())
	}
	select {
	case ch <- dec:
	default:
		return errors.New("duplicate approval decision received")
	}
	return nil
}

func (b *ApprovalBridge) Close(err error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return
	}
	b.closed = true
	b.closeErr = err
	for _, ch := range b.pending {
		close(ch)
	}
	b.pending = make(map[string]chan *agentpb.ApprovalDecision)
	close(b.events)
}
