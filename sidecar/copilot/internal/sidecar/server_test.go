package sidecar

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"sync"
	"testing"
	"time"
)

// harness runs a Server over pipes, like the Rust client does over stdio.
type harness struct {
	t     *testing.T
	in    *io.PipeWriter
	out   *bufio.Reader
	done  chan error
	exits chan int
}

func start(t *testing.T, opts Options) *harness {
	t.Helper()
	inR, inW := io.Pipe()
	outR, outW := io.Pipe()
	h := &harness{t: t, in: inW, out: bufio.NewReader(outR), done: make(chan error, 1), exits: make(chan int, 1)}
	if opts.Log == nil {
		opts.Log = log.New(io.Discard, "", 0)
	}
	if opts.Exit == nil {
		opts.Exit = func(code int) { h.exits <- code }
	}
	s := NewServer(opts)
	go func() {
		err := s.Serve(inR, outW)
		outW.Close()
		h.done <- err
	}()
	t.Cleanup(func() { inW.Close() })
	return h
}

func (h *harness) send(line string) {
	h.t.Helper()
	if _, err := io.WriteString(h.in, line+"\n"); err != nil {
		h.t.Fatalf("write: %v", err)
	}
}

type reply struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Result  json.RawMessage `json:"result"`
	Error   *RPCError       `json:"error"`
}

func (h *harness) read() reply {
	h.t.Helper()
	type res struct {
		line string
		err  error
	}
	ch := make(chan res, 1)
	go func() {
		l, err := h.out.ReadString('\n')
		ch <- res{l, err}
	}()
	select {
	case r := <-ch:
		if r.err != nil {
			h.t.Fatalf("read: %v", r.err)
		}
		var rep reply
		if err := json.Unmarshal([]byte(r.line), &rep); err != nil {
			h.t.Fatalf("reply is not JSON: %q", r.line)
		}
		if rep.JSONRPC != "2.0" {
			h.t.Fatalf("reply lacks jsonrpc 2.0: %q", r.line)
		}
		return rep
	case <-time.After(5 * time.Second):
		h.t.Fatal("timed out waiting for a reply")
	}
	return reply{}
}

var nextID = 0

func (h *harness) call(method string, params any) reply {
	h.t.Helper()
	nextID++
	p, _ := json.Marshal(params)
	h.send(fmt.Sprintf(`{"jsonrpc":"2.0","id":%d,"method":%q,"params":%s}`, nextID, method, p))
	r := h.read()
	if string(r.ID) != fmt.Sprint(nextID) {
		h.t.Fatalf("reply id %s, want %d", r.ID, nextID)
	}
	return r
}

func (h *harness) ok(method string, params any, out any) {
	h.t.Helper()
	r := h.call(method, params)
	if r.Error != nil {
		h.t.Fatalf("%s failed: %v", method, r.Error)
	}
	if out != nil {
		if err := json.Unmarshal(r.Result, out); err != nil {
			h.t.Fatalf("%s result: %v (%s)", method, err, r.Result)
		}
	}
}

func (h *harness) fails(method string, params any, code int) *RPCError {
	h.t.Helper()
	r := h.call(method, params)
	if r.Error == nil {
		h.t.Fatalf("%s succeeded, want error %d: %s", method, code, r.Result)
	}
	if r.Error.Code != code {
		h.t.Fatalf("%s error %d (%s), want %d", method, r.Error.Code, r.Error.Message, code)
	}
	return r.Error
}

func leafPrompt() string {
	return strings.Join([]string{
		"Task: leaf",
		"## file src/a.ts (typescript)",
		"### f1 run",
		"fn run(a) exported, 12 lines",
		"calls: load, save",
		"flow:",
		"  try",
		"    load L3",
		"  catch",
		"    throw L6",
		"### t1 Config",
		"interface Config",
		"### o1 src/a.ts",
		"imports: ./b",
	}, "\n")
}

func TestMockRoundTrips(t *testing.T) {
	h := start(t, Options{Mock: true})

	var st StatusResult
	h.ok("status", map[string]any{}, &st)
	if !st.Mock || !st.SignedIn || st.User == nil || *st.User != "mock-user" || st.Version != Version || st.Live {
		t.Fatalf("status: %+v", st)
	}

	var models ListModelsResult
	h.ok("listModels", map[string]any{}, &models)
	if len(models.Models) != 3 || models.Models[0].ID != "claude-haiku-5.5" || models.Models[0].InputPerM != 0.10 {
		t.Fatalf("models: %+v", models)
	}

	req := CompletionRequest{Model: "claude-haiku-5.5", System: "Task: leaf\nYou write summaries.", Prompt: leafPrompt(), MaxTokens: 1000}
	var c Completion
	h.ok("complete", req, &c)
	if c.Model != req.Model {
		t.Fatalf("model %q", c.Model)
	}
	if c.TokensIn != Tokens(req.System)+Tokens(req.Prompt) || c.TokensOut != Tokens(c.Text) || c.CachedTokensIn != 0 {
		t.Fatalf("token counts: %+v", c)
	}
	var leaf struct {
		Items []struct {
			ID       string   `json:"id"`
			Desc     string   `json:"desc"`
			Algo     []string `json:"algo"`
			Overview string   `json:"overview"`
			Notes    []struct {
				Kind string `json:"kind"`
				Text string `json:"text"`
			} `json:"notes"`
		} `json:"items"`
	}
	if err := json.Unmarshal([]byte(c.Text), &leaf); err != nil {
		t.Fatalf("mock leaf answer is not JSON: %v\n%s", err, c.Text)
	}
	if len(leaf.Items) != 3 || leaf.Items[0].ID != "f1" || leaf.Items[1].ID != "t1" || leaf.Items[2].ID != "o1" {
		t.Fatalf("leaf items: %+v", leaf.Items)
	}
	f := leaf.Items[0]
	if len(f.Algo) < 3 || len(f.Algo) > 7 || !strings.HasPrefix(f.Desc, "Handles run") || len(f.Notes) != 2 || f.Notes[1].Kind != "warn" {
		t.Fatalf("fn item: %+v", f)
	}
	if leaf.Items[2].Overview == "" {
		t.Fatal("no overview")
	}
	for _, s := range append(f.Algo, f.Desc, leaf.Items[2].Overview) {
		if strings.Contains(s, "!") {
			t.Fatalf("exclamation mark in %q", s)
		}
	}

	// The same system message again is served from the simulated prompt cache.
	var c2 Completion
	h.ok("complete", req, &c2)
	if c2.CachedTokensIn != Tokens(req.System) || c2.Text != c.Text {
		t.Fatalf("second completion: %+v", c2)
	}

	var cl Completion
	h.ok("complete", CompletionRequest{Model: "claude-sonnet-5.5", System: "Task: cluster", Prompt: "### m1 src/engine\ndir: src/engine\n- src/engine/run.ts: Runs things.\n"}, &cl)
	var cluster struct {
		Items []struct {
			ID, Name, Role   string
			Responsibilities []string
		} `json:"items"`
	}
	if err := json.Unmarshal([]byte(cl.Text), &cluster); err != nil || len(cluster.Items) != 1 || cluster.Items[0].Name != "Engine" || len(cluster.Items[0].Responsibilities) != 1 {
		t.Fatalf("cluster answer %s (%v)", cl.Text, err)
	}

	var rp Completion
	h.ok("complete", CompletionRequest{System: "Task: repo", Prompt: "### m1 Engine\n### m2 Ui\n"}, &rp)
	var repo struct {
		Overview string
		Layers   []struct {
			Name    string
			Modules []string
		}
	}
	if err := json.Unmarshal([]byte(rp.Text), &repo); err != nil || repo.Overview == "" || len(repo.Layers[0].Modules) != 2 || rp.Model != "claude-haiku-5.5" {
		t.Fatalf("repo answer %s (%v)", rp.Text, err)
	}

	var so SignInPollResult
	h.ok("signOut", map[string]any{}, &so)
	if so.SignedIn {
		t.Fatal("still signed in")
	}
	h.ok("status", nil, &st)
	if st.SignedIn {
		t.Fatal("status says signed in after signOut")
	}
	h.fails("complete", req, CodeNotSignedIn)
	h.fails("listModels", nil, CodeNotSignedIn)

	var si SignInResult
	h.ok("signIn", map[string]any{}, &si)
	if si.UserCode == "" || si.VerificationURI == "" || si.ExpiresIn <= 0 {
		t.Fatalf("signIn: %+v", si)
	}
	var poll SignInPollResult
	h.ok("signInPoll", map[string]any{}, &poll)
	if !poll.SignedIn || poll.User == nil || *poll.User != "mock-user" {
		t.Fatalf("signInPoll: %+v", poll)
	}
	h.ok("complete", req, &c)

	h.ok("shutdown", map[string]any{}, nil)
	select {
	case err := <-h.done:
		if err != nil {
			t.Fatalf("serve: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("server did not stop after shutdown")
	}
}

func TestMalformedInput(t *testing.T) {
	h := start(t, Options{Mock: true})

	h.send(`{not json`)
	r := h.read()
	if r.Error == nil || r.Error.Code != CodeParseError || string(r.ID) != "null" {
		t.Fatalf("parse error reply: %+v", r)
	}
	h.send(`{"id":1,"method":"status"}`)
	if r = h.read(); r.Error == nil || r.Error.Code != CodeInvalidRequest {
		t.Fatalf("missing jsonrpc: %+v", r)
	}
	h.send(`{"jsonrpc":"2.0","id":2}`)
	if r = h.read(); r.Error == nil || r.Error.Code != CodeInvalidRequest {
		t.Fatalf("missing method: %+v", r)
	}
	h.send(`{"jsonrpc":"2.0","id":3,"method":7}`)
	if r = h.read(); r.Error == nil || r.Error.Code != CodeInvalidRequest {
		t.Fatalf("numeric method: %+v", r)
	}
	h.send(`[{"jsonrpc":"2.0","id":4,"method":"status"}]`)
	if r = h.read(); r.Error == nil || r.Error.Code != CodeInvalidRequest {
		t.Fatalf("batch: %+v", r)
	}
	h.send(`"just a string"`)
	if r = h.read(); r.Error == nil || r.Error.Code != CodeInvalidRequest {
		t.Fatalf("string: %+v", r)
	}
	h.send(``) // blank lines are ignored
	h.fails("noSuchMethod", nil, CodeMethodNotFound)
	h.fails("complete", map[string]any{"prompt": 5}, CodeInvalidParams)
	h.fails("complete", map[string]any{"model": "x"}, CodeInvalidParams)
	h.fails("signIn", []int{1}, CodeInvalidParams)

	// A notification gets no reply; the next request still works.
	h.send(`{"jsonrpc":"2.0","method":"status"}`)
	var st StatusResult
	h.ok("status", nil, &st)

	// String ids are echoed as given.
	h.send(`{"jsonrpc":"2.0","id":"abc","method":"status"}`)
	if r = h.read(); string(r.ID) != `"abc"` || r.Error != nil {
		t.Fatalf("string id: %+v", r)
	}

	// EOF ends the server cleanly.
	h.in.Close()
	select {
	case err := <-h.done:
		if err != nil {
			t.Fatalf("serve: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("server did not stop at EOF")
	}
}

func TestShutdownWaitsAndStopsReading(t *testing.T) {
	h := start(t, Options{Mock: true})
	h.send(`{"jsonrpc":"2.0","id":1,"method":"complete","params":{"model":"m","system":"","prompt":"hello"}}` + "\n" +
		`{"jsonrpc":"2.0","id":2,"method":"shutdown","params":{}}` + "\n" +
		`{"jsonrpc":"2.0","id":3,"method":"status","params":{}}`)
	first, second := h.read(), h.read()
	if string(first.ID) != "1" || first.Error != nil {
		t.Fatalf("in-flight request: %+v", first)
	}
	if string(second.ID) != "2" || string(second.Result) != "{}" {
		t.Fatalf("shutdown reply: %+v", second)
	}
	select {
	case err := <-h.done:
		if err != nil {
			t.Fatalf("serve: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("server did not stop")
	}
}

func TestMockCrashPrompt(t *testing.T) {
	h := start(t, Options{Mock: true})
	h.send(`{"jsonrpc":"2.0","id":1,"method":"complete","params":{"prompt":"` + MockCrashPrompt + `"}}`)
	select {
	case code := <-h.exits:
		if code != 3 {
			t.Fatalf("exit code %d", code)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("mock did not exit")
	}
}

func TestLiveWithoutSDK(t *testing.T) {
	t.Setenv("ISOLINE_GITHUB_CLIENT_ID", "")
	h := start(t, Options{Mock: false, Store: &MemoryStore{}})
	var st StatusResult
	h.ok("status", nil, &st)
	if st.Mock || st.SignedIn || st.Live {
		t.Fatalf("status: %+v", st)
	}
	e := h.fails("signIn", map[string]any{}, CodeNoClientID)
	if !strings.Contains(e.Message, "client id") {
		t.Fatalf("message: %s", e.Message)
	}
	h.fails("complete", CompletionRequest{Prompt: "x"}, CodeLiveUnavailable)
	h.fails("listModels", nil, CodeLiveUnavailable)
	h.fails("signInPoll", nil, CodeNoPendingSignIn)
}

// fakeGitHub answers the device-flow endpoints without a network.
type fakeGitHub struct {
	mu    sync.Mutex
	polls int
}

func (f *fakeGitHub) RoundTrip(r *http.Request) (*http.Response, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	body := ""
	switch r.URL.Path {
	case "/login/device/code":
		_ = r.ParseForm()
		if r.Form.Get("client_id") != "client-123" {
			body = `{"error":"bad_client"}`
		} else {
			body = `{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":1}`
		}
	case "/login/oauth/access_token":
		f.polls++
		if f.polls == 1 {
			body = `{"error":"authorization_pending"}`
		} else {
			body = `{"access_token":"gho_test","token_type":"bearer"}`
		}
	case "/user":
		if r.Header.Get("Authorization") != "Bearer gho_test" {
			return &http.Response{StatusCode: 401, Body: io.NopCloser(strings.NewReader("{}")), Header: http.Header{}}, nil
		}
		body = `{"login":"octocat"}`
	default:
		return &http.Response{StatusCode: 404, Body: io.NopCloser(strings.NewReader("")), Header: http.Header{}}, nil
	}
	return &http.Response{StatusCode: 200, Body: io.NopCloser(strings.NewReader(body)), Header: http.Header{"Content-Type": {"application/json"}}}, nil
}

func TestDeviceFlowSignIn(t *testing.T) {
	store := &MemoryStore{}
	now := time.Unix(1_000_000, 0)
	flow := &DeviceFlow{
		HTTP:      &http.Client{Transport: &fakeGitHub{}},
		GitHubURL: "https://github.example",
		APIURL:    "https://api.github.example",
		now:       func() time.Time { return now },
	}
	h := start(t, Options{Mock: false, Store: store, Flow: flow, ClientID: "client-123"})
	var si SignInResult
	h.ok("signIn", map[string]any{}, &si)
	if si.UserCode != "ABCD-1234" || si.ExpiresIn != 900 {
		t.Fatalf("signIn: %+v", si)
	}
	var p SignInPollResult
	h.ok("signInPoll", nil, &p) // authorization_pending
	if p.SignedIn {
		t.Fatal("signed in too early")
	}
	h.ok("signInPoll", nil, &p) // inside the interval: no request, still pending
	if p.SignedIn {
		t.Fatal("polled inside the interval")
	}
	now = now.Add(2 * time.Second)
	h.ok("signInPoll", nil, &p)
	if !p.SignedIn || p.User == nil || *p.User != "octocat" {
		t.Fatalf("poll: %+v", p)
	}
	c, _ := store.Load()
	if c == nil || c.Token != "gho_test" || c.User != "octocat" {
		t.Fatalf("stored: %+v", c)
	}
	var st StatusResult
	h.ok("status", nil, &st)
	if !st.SignedIn || *st.User != "octocat" {
		t.Fatalf("status: %+v", st)
	}
	// Signed in, but this build has no SDK.
	h.fails("complete", CompletionRequest{Prompt: "x"}, CodeLiveUnavailable)

	// A wrong client id is reported as an upstream error.
	h.fails("signIn", SignInParams{ClientID: "other"}, CodeUpstream)

	// An expired code is reported and clears the pending sign-in.
	h.ok("signIn", map[string]any{}, &si)
	now = now.Add(time.Hour)
	h.fails("signInPoll", nil, CodeSignInExpired)
}

func TestMockStopsAtMaxTokens(t *testing.T) {
	m := NewMockBackend(nil)
	c, err := m.Complete(context.Background(), "", CompletionRequest{Prompt: leafPrompt(), MaxTokens: 5})
	if err != nil || c.TokensOut != 5 || len([]rune(c.Text)) != 20 {
		t.Fatalf("completion %+v (%v)", c, err)
	}
}

func TestTokens(t *testing.T) {
	cases := map[string]uint64{"": 0, "a": 1, "abcd": 1, "abcde": 2, "ééééé": 2}
	for s, want := range cases {
		if got := Tokens(s); got != want {
			t.Fatalf("Tokens(%q) = %d, want %d", s, got, want)
		}
	}
}
