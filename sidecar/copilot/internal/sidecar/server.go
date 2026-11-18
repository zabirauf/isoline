package sidecar

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"sync"
	"time"
)

// Backend reaches a model. Mock mode uses MockBackend; live mode uses the Copilot SDK
// backend, which only the live build (sidecar/copilot/live) links in.
type Backend interface {
	ListModels(ctx context.Context, token string) ([]ModelInfo, error)
	Complete(ctx context.Context, token string, req CompletionRequest) (*Completion, error)
	Close() error
}

// LiveConfig is passed to a LiveFactory.
type LiveConfig struct {
	// DataDir is where the Copilot runtime may keep session state (never the repo).
	DataDir string
	Log     *log.Logger
}

// LiveFactory builds the live backend. Nil means live mode is not available in this build.
type LiveFactory func(cfg LiveConfig) (Backend, error)

// DefaultClientID is Isoline's GitHub OAuth app, used for device-flow sign-in. A client id
// is public by design: the device flow needs no client secret.
const DefaultClientID = "Ov23liMTukjjf5HWlgAg"

// defaultClientID is ISOLINE_GITHUB_CLIENT_ID when set (even empty, which turns sign-in off),
// else DefaultClientID.
func defaultClientID() string {
	if v, ok := os.LookupEnv("ISOLINE_GITHUB_CLIENT_ID"); ok {
		return v
	}
	return DefaultClientID
}

// Options configure a Server.
type Options struct {
	Mock     bool
	Live     LiveFactory
	LiveCfg  LiveConfig
	Store    TokenStore  // nil: MemoryStore in mock mode, KeyringStore in live mode
	Flow     *DeviceFlow // nil: GitHub's endpoints
	ClientID string      // default client id (see defaultClientID)
	Log      *log.Logger // nil: stderr
	Exit     func(int)   // nil: os.Exit
	// ShutdownWait bounds how long shutdown waits for in-flight requests.
	ShutdownWait time.Duration
}

// Server holds the sidecar state. Requests are handled concurrently; replies are written
// one line at a time.
type Server struct {
	opts    Options
	log     *log.Logger
	store   TokenStore
	flow    *DeviceFlow
	backend Backend
	liveErr error

	mu      sync.Mutex
	loaded  bool
	creds   *Credentials
	pending *DevicePending

	outMu sync.Mutex
	out   *bufio.Writer
}

// NewServer builds a server from options.
func NewServer(opts Options) *Server {
	s := &Server{opts: opts, log: opts.Log}
	if s.log == nil {
		s.log = log.New(os.Stderr, "copilot-sidecar: ", log.LstdFlags)
	}
	if opts.Exit == nil {
		s.opts.Exit = os.Exit
	}
	if opts.ShutdownWait == 0 {
		s.opts.ShutdownWait = 10 * time.Second
	}
	s.flow = opts.Flow
	if s.flow == nil {
		s.flow = &DeviceFlow{
			HTTP:      &http.Client{Timeout: 30 * time.Second},
			GitHubURL: "https://github.com",
			APIURL:    "https://api.github.com",
			Scope:     "read:user",
		}
	}
	s.store = opts.Store
	if opts.Mock {
		if s.store == nil {
			s.store = &MemoryStore{}
			// Mock mode starts signed in so the pipeline can run without a sign-in step.
			_ = s.store.Save(Credentials{Token: "mock-token", User: "mock-user"})
		}
		s.backend = NewMockBackend(s.opts.Exit)
	} else {
		if s.store == nil {
			s.store = KeyringStore{}
		}
		if opts.Live == nil {
			s.liveErr = Errorf(CodeLiveUnavailable,
				"live Copilot mode is not built into this sidecar; build it with sidecar/copilot/build.sh (the Copilot SDK needs Go 1.26), or run with --mock")
		} else {
			cfg := opts.LiveCfg
			cfg.Log = s.log
			b, err := opts.Live(cfg)
			if err != nil {
				s.liveErr = Errorf(CodeLiveUnavailable, "the Copilot SDK backend failed to start: %v", err)
			} else {
				s.backend = b
			}
		}
	}
	return s
}

// Serve reads requests from in until EOF or shutdown. It returns nil on a clean exit.
func (s *Server) Serve(in io.Reader, out io.Writer) error {
	s.out = bufio.NewWriter(out)
	r := bufio.NewReaderSize(in, 1<<16)
	var wg sync.WaitGroup
	defer func() {
		if s.backend != nil {
			_ = s.backend.Close()
		}
	}()
	for {
		line, err := r.ReadBytes('\n')
		if len(bytes.TrimSpace(line)) > 0 {
			req, perr := parseRequest(line)
			if perr != nil {
				s.write(rpcResponse{JSONRPC: "2.0", ID: json.RawMessage("null"), Error: perr})
			} else if req.Method == "shutdown" {
				s.waitFor(&wg)
				if req.ID != nil {
					s.write(rpcResponse{JSONRPC: "2.0", ID: req.ID, Result: struct{}{}})
				}
				s.log.Printf("shutdown requested")
				return nil
			} else {
				wg.Add(1)
				go func(req rpcRequest) {
					defer wg.Done()
					s.handle(req)
				}(req)
			}
		}
		if err != nil {
			s.waitFor(&wg)
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}
	}
}

func (s *Server) waitFor(wg *sync.WaitGroup) {
	done := make(chan struct{})
	go func() { wg.Wait(); close(done) }()
	select {
	case <-done:
	case <-time.After(s.opts.ShutdownWait):
		s.log.Printf("in-flight requests did not finish within %s", s.opts.ShutdownWait)
	}
}

func parseRequest(line []byte) (rpcRequest, *RPCError) {
	var req rpcRequest
	trimmed := bytes.TrimSpace(line)
	if len(trimmed) > 0 && trimmed[0] == '[' {
		return req, Errorf(CodeInvalidRequest, "batch requests are not supported")
	}
	if err := json.Unmarshal(trimmed, &req); err != nil {
		var syn *json.SyntaxError
		if errors.As(err, &syn) || !json.Valid(trimmed) {
			return req, Errorf(CodeParseError, "parse error: %v", err)
		}
		return req, Errorf(CodeInvalidRequest, "invalid request: %v", err)
	}
	if req.JSONRPC != "2.0" || req.Method == "" {
		return req, Errorf(CodeInvalidRequest, "invalid request: jsonrpc must be \"2.0\" and method must be set")
	}
	return req, nil
}

func (s *Server) write(resp rpcResponse) {
	b, err := json.Marshal(resp)
	if err != nil {
		b, _ = json.Marshal(rpcResponse{JSONRPC: "2.0", ID: resp.ID, Error: Errorf(CodeInternal, "could not encode the reply: %v", err)})
	}
	s.outMu.Lock()
	defer s.outMu.Unlock()
	_, _ = s.out.Write(b)
	_ = s.out.WriteByte('\n')
	_ = s.out.Flush()
}

func (s *Server) handle(req rpcRequest) {
	result, err := s.dispatch(req)
	if req.ID == nil { // notification: no reply
		return
	}
	resp := rpcResponse{JSONRPC: "2.0", ID: req.ID}
	if err != nil {
		var re *RPCError
		if !errors.As(err, &re) {
			re = Errorf(CodeInternal, "%v", err)
		}
		resp.Error = re
	} else {
		resp.Result = result
	}
	s.write(resp)
}

func decodeParams(raw json.RawMessage, v any) error {
	if len(raw) == 0 || string(raw) == "null" {
		return nil
	}
	if err := json.Unmarshal(raw, v); err != nil {
		return Errorf(CodeInvalidParams, "invalid params: %v", err)
	}
	return nil
}

func (s *Server) credentials() (*Credentials, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if !s.loaded {
		c, err := s.store.Load()
		if err != nil {
			return nil, Errorf(CodeKeychain, "reading the token from the OS keychain failed: %v", err)
		}
		s.creds = c
		s.loaded = true
	}
	return s.creds, nil
}

func strPtr(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}

func (s *Server) dispatch(req rpcRequest) (any, error) {
	ctx := context.Background()
	switch req.Method {
	case "status":
		c, err := s.credentials()
		if err != nil {
			return nil, err
		}
		st := StatusResult{Mock: s.opts.Mock, Version: Version, Live: s.opts.Live != nil}
		if c != nil {
			st.SignedIn = true
			st.User = strPtr(c.User)
		}
		return st, nil

	case "signIn":
		var p SignInParams
		if err := decodeParams(req.Params, &p); err != nil {
			return nil, err
		}
		if s.opts.Mock {
			s.mu.Lock()
			s.pending = &DevicePending{ClientID: "mock", DeviceCode: "mock", Expires: time.Now().Add(15 * time.Minute)}
			s.mu.Unlock()
			return SignInResult{UserCode: "MOCK-0000", VerificationURI: "https://github.com/login/device", ExpiresIn: 900}, nil
		}
		clientID := p.ClientID
		if clientID == "" {
			clientID = s.opts.ClientID
		}
		if clientID == "" {
			return nil, Errorf(CodeNoClientID,
				"sign-in needs a GitHub OAuth app client id: pass clientId to signIn or set ISOLINE_GITHUB_CLIENT_ID")
		}
		pending, res, err := s.flow.Start(ctx, clientID)
		if err != nil {
			return nil, err
		}
		s.mu.Lock()
		s.pending = pending
		s.mu.Unlock()
		return res, nil

	case "signInPoll":
		s.mu.Lock()
		pending := s.pending
		s.mu.Unlock()
		if pending == nil {
			c, err := s.credentials()
			if err != nil {
				return nil, err
			}
			if c != nil {
				return SignInPollResult{SignedIn: true, User: strPtr(c.User)}, nil
			}
			return nil, Errorf(CodeNoPendingSignIn, "no sign-in is in progress; call signIn first")
		}
		var outcome PollOutcome
		if s.opts.Mock {
			outcome = PollOutcome{Done: true, Token: "mock-token", User: "mock-user"}
		} else {
			var err error
			outcome, err = s.flow.Poll(ctx, pending)
			if err != nil {
				var re *RPCError
				if errors.As(err, &re) && (re.Code == CodeSignInExpired || re.Code == CodeSignInDenied) {
					s.mu.Lock()
					s.pending = nil
					s.mu.Unlock()
				}
				return nil, err
			}
		}
		if !outcome.Done {
			return SignInPollResult{SignedIn: false}, nil
		}
		c := Credentials{Token: outcome.Token, User: outcome.User}
		if err := s.store.Save(c); err != nil {
			return nil, Errorf(CodeKeychain, "storing the token in the OS keychain failed: %v", err)
		}
		s.mu.Lock()
		s.creds, s.loaded, s.pending = &c, true, nil
		s.mu.Unlock()
		s.log.Printf("signed in as %s", c.User)
		return SignInPollResult{SignedIn: true, User: strPtr(c.User)}, nil

	case "signOut":
		if err := s.store.Delete(); err != nil {
			return nil, Errorf(CodeKeychain, "removing the token from the OS keychain failed: %v", err)
		}
		s.mu.Lock()
		s.creds, s.loaded, s.pending = nil, true, nil
		s.mu.Unlock()
		return SignInPollResult{SignedIn: false}, nil

	case "listModels":
		token, err := s.ready()
		if err != nil {
			return nil, err
		}
		models, err := s.backend.ListModels(ctx, token)
		if err != nil {
			return nil, upstream(err)
		}
		if models == nil {
			models = []ModelInfo{}
		}
		return ListModelsResult{Models: models}, nil

	case "complete":
		var p CompletionRequest
		if err := decodeParams(req.Params, &p); err != nil {
			return nil, err
		}
		if p.Prompt == "" {
			return nil, Errorf(CodeInvalidParams, "invalid params: prompt is empty")
		}
		token, err := s.ready()
		if err != nil {
			return nil, err
		}
		c, err := s.backend.Complete(ctx, token, p)
		if err != nil {
			return nil, upstream(err)
		}
		return c, nil

	default:
		return nil, Errorf(CodeMethodNotFound, "method not found: %s", req.Method)
	}
}

// ready returns the token when a model call can go ahead.
func (s *Server) ready() (string, error) {
	if s.backend == nil {
		return "", s.liveErr
	}
	c, err := s.credentials()
	if err != nil {
		return "", err
	}
	if c == nil {
		return "", Errorf(CodeNotSignedIn, "not signed in")
	}
	return c.Token, nil
}

func upstream(err error) error {
	var re *RPCError
	if errors.As(err, &re) {
		return re
	}
	return Errorf(CodeUpstream, "%v", err)
}

// Main is the command-line entry point shared by the mock-only and the live build.
func Main(args []string, live LiveFactory) int {
	mock := os.Getenv("ISOLINE_SIDECAR_MOCK") == "1"
	for _, a := range args {
		switch a {
		case "--mock":
			mock = true
		case "--version":
			fmt.Println(Version)
			return 0
		case "-h", "--help":
			fmt.Fprintln(os.Stderr, "usage: copilot-sidecar [--mock] [--version]\nJSON-RPC 2.0 on stdin/stdout, one message per line. Logs go to stderr.")
			return 0
		default:
			fmt.Fprintf(os.Stderr, "unknown argument %q\n", a)
			return 2
		}
	}
	dataDir := os.Getenv("ISOLINE_DATA_DIR")
	if dataDir == "" {
		if d, err := os.UserConfigDir(); err == nil {
			dataDir = d + string(os.PathSeparator) + "isoline"
		}
	}
	s := NewServer(Options{
		Mock:     mock,
		Live:     live,
		LiveCfg:  LiveConfig{DataDir: dataDir},
		ClientID: defaultClientID(),
	})
	s.log.Printf("started (mock=%v, live=%v, version %s)", mock, live != nil, Version)
	if err := s.Serve(os.Stdin, os.Stdout); err != nil {
		s.log.Printf("stopped: %v", err)
		return 1
	}
	return 0
}
