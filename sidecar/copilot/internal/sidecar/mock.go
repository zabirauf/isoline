package sidecar

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"sync"
	"unicode/utf8"
)

// MockCrashPrompt makes the mock backend exit the process, so clients can test restarts.
// It only works in mock mode.
const MockCrashPrompt = "__isoline_mock_crash__"

// Tokens counts tokens the way mock mode does: ceil(chars / 4).
func Tokens(s string) uint64 {
	n := uint64(utf8.RuneCountInString(s))
	return (n + 3) / 4
}

// MockModels are the models mock mode lists. Prices are the analysis plan's defaults
// (USD per million tokens; cached input is a tenth of fresh input).
var MockModels = []ModelInfo{
	{ID: "claude-haiku-5.5", Name: "Claude Haiku 5.5", InputPerM: 0.10, OutputPerM: 0.50, CachedInputPerM: 0.01},
	{ID: "claude-sonnet-5.5", Name: "Claude Sonnet 5.5", InputPerM: 2.00, OutputPerM: 10.00, CachedInputPerM: 0.20},
	{ID: "claude-opus-5.5", Name: "Claude Opus 5.5", InputPerM: 4.00, OutputPerM: 20.00, CachedInputPerM: 0.40},
}

// MockBackend answers without any network call. Replies are deterministic and shaped like
// the JSON the iso-ai prompts ask for, so the whole pipeline can run against it.
type MockBackend struct {
	mu   sync.Mutex
	seen map[[32]byte]bool // system messages seen, to simulate provider prompt caching
	exit func(code int)
}

// NewMockBackend returns a mock backend. exit is called for MockCrashPrompt.
func NewMockBackend(exit func(int)) *MockBackend {
	return &MockBackend{seen: map[[32]byte]bool{}, exit: exit}
}

func (m *MockBackend) ListModels(context.Context, string) ([]ModelInfo, error) {
	out := make([]ModelInfo, len(MockModels))
	copy(out, MockModels)
	return out, nil
}

func (m *MockBackend) Close() error { return nil }

func (m *MockBackend) Complete(_ context.Context, _ string, req CompletionRequest) (*Completion, error) {
	if req.Prompt == MockCrashPrompt && m.exit != nil {
		m.exit(3)
	}
	model := req.Model
	if model == "" {
		model = MockModels[0].ID
	}
	text := MockAnswer(req.System, req.Prompt)
	// Like a real model, stop at maxTokens.
	if req.MaxTokens > 0 && Tokens(text) > uint64(req.MaxTokens) {
		r := []rune(text)
		text = string(r[:int(req.MaxTokens)*4])
	}
	sysTokens := Tokens(req.System)
	m.mu.Lock()
	h := sha256.Sum256([]byte(model + "\x00" + req.System))
	var cached uint64
	if m.seen[h] {
		cached = sysTokens
	}
	m.seen[h] = true
	m.mu.Unlock()
	return &Completion{
		Text:           text,
		Model:          model,
		TokensIn:       sysTokens + Tokens(req.Prompt),
		TokensOut:      Tokens(text),
		CachedTokensIn: cached,
	}, nil
}

var (
	taskRe = regexp.MustCompile(`(?m)^Task: (\w+)`)
	itemRe = regexp.MustCompile(`^### (\S+) ?(.*)$`)
)

type mockItem struct {
	id   string
	name string
	body []string
}

func mockItems(prompt string) []mockItem {
	var items []mockItem
	var cur *mockItem
	for _, line := range strings.Split(prompt, "\n") {
		if m := itemRe.FindStringSubmatch(line); m != nil {
			items = append(items, mockItem{id: m[1], name: strings.TrimSpace(m[2])})
			cur = &items[len(items)-1]
			continue
		}
		if strings.HasPrefix(line, "## ") {
			cur = nil
			continue
		}
		if cur != nil {
			cur.body = append(cur.body, line)
		}
	}
	return items
}

func field(body []string, key string) string {
	for _, l := range body {
		t := strings.TrimSpace(l)
		if strings.HasPrefix(t, key+":") {
			return strings.TrimSpace(strings.TrimPrefix(t, key+":"))
		}
	}
	return ""
}

func splitList(s string) []string {
	var out []string
	for _, p := range strings.Split(s, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

func lastSegment(s string) string {
	s = strings.TrimRight(s, "/")
	if i := strings.LastIndex(s, "/"); i >= 0 {
		s = s[i+1:]
	}
	if s == "" {
		return "Root"
	}
	return strings.ToUpper(s[:1]) + s[1:]
}

// MockAnswer builds the deterministic reply for a prompt. The task comes from a
// "Task: <kind>" line in the system message or the prompt; items are "### <id> <name>"
// headers in the prompt.
func MockAnswer(system, prompt string) string {
	task := ""
	if m := taskRe.FindStringSubmatch(prompt); m != nil {
		task = m[1]
	} else if m := taskRe.FindStringSubmatch(system); m != nil {
		task = m[1]
	}
	items := mockItems(prompt)
	var out any
	switch task {
	case "leaf":
		var list []map[string]any
		for _, it := range items {
			switch {
			case strings.HasPrefix(it.id, "f"):
				list = append(list, mockFn(it))
			case strings.HasPrefix(it.id, "t"):
				list = append(list, map[string]any{"id": it.id,
					"desc": fmt.Sprintf("Describes the shape of %s values that this file passes around.", it.name)})
			case strings.HasPrefix(it.id, "o"):
				list = append(list, map[string]any{"id": it.id,
					"overview": fmt.Sprintf("Declares the functions and types of %s and the imports they rely on. Most of its work happens in the exported entry points.", it.name)})
			}
		}
		out = map[string]any{"items": nonNil(list)}
	case "cluster":
		var list []map[string]any
		for _, it := range items {
			dir := field(it.body, "dir")
			if dir == "" {
				dir = it.name
			}
			var resp []string
			for _, l := range it.body {
				t := strings.TrimSpace(l)
				if strings.HasPrefix(t, "- ") && len(resp) < 3 {
					p := strings.TrimPrefix(t, "- ")
					if i := strings.Index(p, ":"); i > 0 {
						p = p[:i]
					}
					resp = append(resp, fmt.Sprintf("Keeps the code in %s.", p))
				}
			}
			if len(resp) == 0 {
				resp = []string{fmt.Sprintf("Keeps the code under %s.", dir)}
			}
			list = append(list, map[string]any{
				"id":               it.id,
				"name":             lastSegment(dir),
				"role":             fmt.Sprintf("Groups the code under %s that works together.", dir),
				"responsibilities": resp,
			})
		}
		out = map[string]any{"items": nonNil(list)}
	case "repo":
		ids := []string{}
		for _, it := range items {
			ids = append(ids, it.id)
		}
		out = map[string]any{
			"overview": fmt.Sprintf("The repository is split into %d modules. Each module groups the files of one directory, and the module graph shows how they depend on each other.", len(items)),
			"layers":   []map[string]any{{"name": "Core", "modules": ids}},
		}
	case "review":
		title := "the change"
		if m := regexp.MustCompile(`(?m)^Title: (.*)$`).FindStringSubmatch(prompt); m != nil {
			title = strings.TrimSpace(m[1])
		}
		var files []mockItem
		mods := []map[string]any{}
		for _, it := range items {
			if strings.HasPrefix(it.id, "f") {
				files = append(files, it)
			} else if strings.HasPrefix(it.id, "m") {
				mods = append(mods, map[string]any{"id": it.id, "text": fmt.Sprintf("Changes the code of %s.", it.name)})
			}
		}
		notes := []map[string]any{}
		if len(files) > 0 {
			name := strings.Fields(files[0].name)
			first := files[0].name
			if len(name) > 0 {
				first = name[0]
			}
			notes = append(notes, map[string]any{"kind": "risk", "text": fmt.Sprintf("Changes %s the most. Read its diff first.", first), "file": files[0].id})
		}
		notes = append(notes, map[string]any{"kind": "check", "text": "Run the tests of the changed modules."})
		out = map[string]any{
			"summary": fmt.Sprintf("The change is about \"%s\". It edits %d files in %d modules.", title, len(files), len(mods)),
			"notes":   notes,
			"modules": mods,
		}
	default:
		return "Mock reply for a prompt of " + fmt.Sprint(Tokens(prompt)) + " tokens."
	}
	b, _ := json.Marshal(out)
	return string(b)
}

func nonNil(l []map[string]any) []map[string]any {
	if l == nil {
		return []map[string]any{}
	}
	return l
}

func mockFn(it mockItem) map[string]any {
	calls := splitList(field(it.body, "calls"))
	var algo []string
	for _, c := range calls {
		if len(algo) >= 4 {
			break
		}
		algo = append(algo, fmt.Sprintf("Calls %s and keeps what it returns for the next step.", c))
	}
	fillers := []string{
		"Reads the arguments it was given and checks that they are usable.",
		"Combines the intermediate values into one result.",
		"Returns the result to the caller.",
	}
	for i := 0; len(algo) < 3; i++ {
		algo = append(algo, fillers[i])
	}
	body := strings.Join(it.body, "\n")
	notes := []map[string]string{}
	if strings.Contains(body, "catch") {
		notes = append(notes, map[string]string{"kind": "info", "text": "Errors raised inside the try block are caught here, so callers never see them."})
	}
	if strings.Contains(body, "throw") {
		notes = append(notes, map[string]string{"kind": "warn", "text": "Throws when a check fails, and the error reaches the caller unchanged."})
	}
	return map[string]any{
		"id":    it.id,
		"desc":  fmt.Sprintf("Handles %s for its callers and returns the outcome.", it.name),
		"algo":  algo,
		"notes": notes,
	}
}
