package agent

import (
	"context"
	"encoding/json"
	"fmt"
	"os"

	"github.com/microsoft/agent-framework-go/agent"
	"github.com/microsoft/agent-framework-go/agent/harness/toolautocall"
	"github.com/microsoft/agent-framework-go/provider/openaiprovider"
	"github.com/microsoft/agent-framework-go/tool"
	"github.com/microsoft/agent-framework-go/tool/functool"
	"github.com/openai/openai-go/v3"
	"github.com/openai/openai-go/v3/option"

	"sidecar-go/internal/lens"
)

const (
	DefaultBaseURL = "http://bigbertha:11434/v1"
	DefaultModel   = "glm-5.3-flash:cloud"

	LensInstructions = `You are an agent inside Chamber. When the user asks for a dashboard, view,
panel, report, form, command center, or change to the current UI, use the
lens_upsert tool. Create a complete, self-contained HTML document. Use a
short lowercase ID containing only letters, numbers, and hyphens. Canvas
buttons may call window.canvas.sendAction(action, data) to send intent back
to you. Do not use external scripts, styles, images, or network requests.
Use the Chamber classes ch-page, ch-grid, ch-card, ch-button,
ch-button-secondary, ch-input, ch-table, ch-badge, and ch-muted.`
)

type LensUpsertArgs struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	Icon string `json:"icon"`
	HTML string `json:"html"`
}

type LensListener func(snapshot lens.LensSnapshot)

func BuildAgent(mindRoot string, onLensChanged LensListener) (*agent.Agent, error) {
	baseURL := os.Getenv("OLLAMA_BASE_URL")
	if baseURL == "" {
		baseURL = DefaultBaseURL
	}
	model := os.Getenv("CHAMBER_MODEL")
	if model == "" {
		model = DefaultModel
	}

	client := openai.NewClient(
		option.WithBaseURL(baseURL),
		option.WithAPIKey("ollama"),
	)

	lensTool := functool.MustNew(functool.Config{
		Name:        "lens_upsert",
		Description: "Create or replace a sandboxed Canvas Lens in Chamber.",
	}, func(ctx context.Context, in LensUpsertArgs) (string, error) {
		if err := lens.Validate(in.ID, in.Name, in.Icon, in.HTML); err != nil {
			return "", err
		}
		if err := lens.Write(mindRoot, in.ID, in.Name, in.Icon, in.HTML); err != nil {
			return "", fmt.Errorf("could not write lens: %w", err)
		}
		if onLensChanged != nil {
			onLensChanged(lens.LensSnapshot{
				ID:   in.ID,
				Name: in.Name,
				Icon: in.Icon,
				HTML: in.HTML,
			})
		}
		res, _ := json.Marshal(map[string]any{
			"ok":      true,
			"id":      in.ID,
			"message": "Lens saved and displayed",
		})
		return string(res), nil
	})

	a := openaiprovider.NewChatCompletionsAgent(
		client,
		openaiprovider.AgentConfig{
			Model:        model,
			Instructions: LensInstructions,
			ToolAutoCall: &toolautocall.Config{
				DisableApprovalResponseBinding: true,
			},
			Config: agent.Config{
				Tools: []tool.Tool{tool.ApprovalRequiredFunc(lensTool)},
			},
		},
	)

	return a, nil
}
