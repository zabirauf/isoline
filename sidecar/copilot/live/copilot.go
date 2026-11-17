package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	copilot "github.com/github/copilot-sdk/go"
	"github.com/github/copilot-sdk/go/rpc"

	"isoline/sidecar/copilot/internal/sidecar"
)

// copilotBackend reaches models through the GitHub Copilot SDK. It locks the SDK down so
// it behaves as a plain model: every built-in, MCP and custom tool is excluded, the system
// message is replaced, and the runtime works in a private folder, never the repo.
type copilotBackend struct {
	cfg sidecar.LiveConfig

	mu     sync.Mutex
	client *copilot.Client
	token  string
}

func newCopilotBackend(cfg sidecar.LiveConfig) (sidecar.Backend, error) {
	// The client starts lazily, on the first call that needs a model.
	return &copilotBackend{cfg: cfg}, nil
}

func (b *copilotBackend) runtimeDir() (string, error) {
	base := b.cfg.DataDir
	if base == "" {
		base = os.TempDir()
	}
	dir := filepath.Join(base, "copilot-runtime")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", err
	}
	return dir, nil
}

func (b *copilotBackend) clientFor(ctx context.Context, token string) (*copilot.Client, string, error) {
	dir, err := b.runtimeDir()
	if err != nil {
		return nil, "", err
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.client != nil && b.token == token {
		return b.client, dir, nil
	}
	if b.client != nil {
		_ = b.client.Stop()
		b.client = nil
	}
	c := copilot.NewClient(&copilot.ClientOptions{
		GitHubToken:      token,
		UseLoggedInUser:  copilot.Bool(false),
		Mode:             copilot.ModeEmpty,
		BaseDirectory:    dir,
		WorkingDirectory: dir,
		LogLevel:         "error",
		Env:              append(os.Environ(), "COPILOT_TELEMETRY_DISABLED=true"),
	})
	if err := c.Start(ctx); err != nil {
		return nil, "", fmt.Errorf("starting the Copilot runtime: %w", err)
	}
	b.client, b.token = c, token
	return c, dir, nil
}

func (b *copilotBackend) Close() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.client != nil {
		err := b.client.Stop()
		b.client = nil
		return err
	}
	return nil
}

// usdPerMillion converts a Copilot token price (per billing batch, in AI credits or
// nano-AIU) to USD per million tokens. One AI credit is $0.01.
func usdPerMillion(price *float64, batch *int64, unit rpc.TokenPriceUnit) float64 {
	if price == nil {
		return 0
	}
	n := 1.0
	if batch != nil && *batch > 0 {
		n = float64(*batch)
	}
	credits := *price / n * 1e6
	if unit == rpc.TokenPriceUnitNanoAiu {
		credits /= 1e9
	}
	return credits * 0.01
}

func (b *copilotBackend) ListModels(ctx context.Context, token string) ([]sidecar.ModelInfo, error) {
	ctx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	c, _, err := b.clientFor(ctx, token)
	if err != nil {
		return nil, err
	}
	models, err := c.ListModels(ctx)
	if err != nil {
		return nil, err
	}
	units := map[string]rpc.TokenPriceUnit{}
	if c.RPC != nil && c.RPC.Models != nil {
		if list, err := c.RPC.Models.ListTokenPriceUnits(ctx); err == nil && list != nil {
			for _, u := range list.Units {
				units[u.ModelID] = u.Unit
			}
		} else if err != nil {
			b.cfg.Log.Printf("listing token price units failed, assuming AI credits: %v", err)
		}
	}
	out := make([]sidecar.ModelInfo, 0, len(models))
	for _, m := range models {
		if m.Policy != nil && strings.EqualFold(m.Policy.State, "disabled") {
			continue
		}
		info := sidecar.ModelInfo{ID: m.ID, Name: m.Name}
		if m.Billing != nil && m.Billing.TokenPrices != nil {
			p := m.Billing.TokenPrices
			unit := units[m.ID]
			cached := p.CacheReadPrice
			if cached == nil {
				cached = p.CachePrice
			}
			info.InputPerM = usdPerMillion(p.InputPrice, p.BatchSize, unit)
			info.OutputPerM = usdPerMillion(p.OutputPrice, p.BatchSize, unit)
			info.CachedInputPerM = usdPerMillion(cached, p.BatchSize, unit)
		}
		out = append(out, info)
	}
	return out, nil
}

// denyAll refuses every permission request. No tools are available, so none should come.
func denyAll(copilot.PermissionRequest, copilot.PermissionInvocation) (rpc.PermissionDecision, error) {
	fb := "Isoline runs the model without tools."
	return &rpc.PermissionDecisionReject{Feedback: &fb}, nil
}

func (b *copilotBackend) Complete(ctx context.Context, token string, req sidecar.CompletionRequest) (*sidecar.Completion, error) {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Minute)
	defer cancel()
	c, dir, err := b.clientFor(ctx, token)
	if err != nil {
		return nil, err
	}
	cfg := &copilot.SessionConfig{
		Model:                              req.Model,
		ClientName:                         "isoline",
		SystemMessage:                      &copilot.SystemMessageConfig{Mode: "replace", Content: req.System},
		AvailableTools:                     []string{},
		ExcludedTools:                      copilot.NewToolSet().AddBuiltIn("*").AddMCP("*").AddCustom("*").ToSlice(),
		OnPermissionRequest:                denyAll,
		WorkingDirectory:                   dir,
		Streaming:                          copilot.Bool(false),
		EnableSessionStore:                 copilot.Bool(false),
		EnableConfigDiscovery:              copilot.Bool(false),
		EnableOnDemandInstructionDiscovery: copilot.Bool(false),
		EnableFileHooks:                    copilot.Bool(false),
		EnableHostGitOperations:            copilot.Bool(false),
		EnableSkills:                       copilot.Bool(false),
		SkipCustomInstructions:             copilot.Bool(true),
		InfiniteSessions:                   &copilot.InfiniteSessionConfig{Enabled: copilot.Bool(false)},
	}
	if req.MaxTokens > 0 {
		n := int64(req.MaxTokens)
		cfg.ModelCapabilities = &rpc.ModelCapabilitiesOverride{Limits: &rpc.ModelCapabilitiesOverrideLimits{MaxOutputTokens: &n}}
	}
	session, err := c.CreateSession(ctx, cfg)
	if err != nil {
		return nil, fmt.Errorf("creating a Copilot session: %w", err)
	}
	defer func() {
		_ = session.Disconnect()
		dctx, dcancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer dcancel()
		_ = c.DeleteSession(dctx, session.SessionID)
	}()

	var mu sync.Mutex
	var in, out, cached uint64
	model := req.Model
	unsubscribe := session.On(func(ev copilot.SessionEvent) {
		if u, ok := ev.Data.(*copilot.AssistantUsageData); ok {
			mu.Lock()
			defer mu.Unlock()
			if u.InputTokens != nil {
				in += uint64(*u.InputTokens)
			}
			if u.OutputTokens != nil {
				out += uint64(*u.OutputTokens)
			}
			if u.CacheReadTokens != nil {
				cached += uint64(*u.CacheReadTokens)
			}
			if u.Model != "" {
				model = u.Model
			}
		}
	})
	defer unsubscribe()

	ev, err := session.SendAndWait(ctx, copilot.MessageOptions{Prompt: req.Prompt})
	if err != nil {
		return nil, fmt.Errorf("the model call failed: %w", err)
	}
	if ev == nil {
		return nil, errors.New("the model sent no answer")
	}
	msg, ok := ev.Data.(*copilot.AssistantMessageData)
	if !ok {
		return nil, fmt.Errorf("unexpected final event %T", ev.Data)
	}
	mu.Lock()
	defer mu.Unlock()
	if in == 0 && out == 0 {
		// Usage events are best effort in the SDK; fall back to the mock estimate so every
		// reply still carries token counts.
		in = sidecar.Tokens(req.System) + sidecar.Tokens(req.Prompt)
		out = sidecar.Tokens(msg.Content)
		if msg.OutputTokens != nil {
			out = uint64(*msg.OutputTokens)
		}
	}
	return &sidecar.Completion{
		Text:           msg.Content,
		Model:          model,
		TokensIn:       in,
		TokensOut:      out,
		CachedTokensIn: cached,
	}, nil
}
