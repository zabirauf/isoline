package sidecar

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/zalando/go-keyring"
)

// Credentials is what the sidecar keeps for a signed-in user.
type Credentials struct {
	Token string `json:"token"`
	User  string `json:"user"`
}

// TokenStore keeps the GitHub token. The default store is the OS keychain; the token is
// never written to disk in plain text.
type TokenStore interface {
	Load() (*Credentials, error) // nil, nil when nothing is stored
	Save(c Credentials) error
	Delete() error
}

const (
	keyringService = "isoline-copilot"
	keyringAccount = "github"
)

// KeyringStore stores credentials in the OS keychain (macOS Keychain, Windows Credential
// Manager, Secret Service on Linux).
type KeyringStore struct{}

func (KeyringStore) Load() (*Credentials, error) {
	s, err := keyring.Get(keyringService, keyringAccount)
	if errors.Is(err, keyring.ErrNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var c Credentials
	if err := json.Unmarshal([]byte(s), &c); err != nil || c.Token == "" {
		return nil, nil
	}
	return &c, nil
}

func (KeyringStore) Save(c Credentials) error {
	b, err := json.Marshal(c)
	if err != nil {
		return err
	}
	return keyring.Set(keyringService, keyringAccount, string(b))
}

func (KeyringStore) Delete() error {
	err := keyring.Delete(keyringService, keyringAccount)
	if errors.Is(err, keyring.ErrNotFound) {
		return nil
	}
	return err
}

// MemoryStore keeps credentials in memory only. Mock mode and tests use it.
type MemoryStore struct {
	mu sync.Mutex
	c  *Credentials
}

func (m *MemoryStore) Load() (*Credentials, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.c == nil {
		return nil, nil
	}
	c := *m.c
	return &c, nil
}

func (m *MemoryStore) Save(c Credentials) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.c = &c
	return nil
}

func (m *MemoryStore) Delete() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.c = nil
	return nil
}

// DeviceFlow runs the GitHub OAuth device flow
// (https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow).
type DeviceFlow struct {
	HTTP      *http.Client
	GitHubURL string // https://github.com
	APIURL    string // https://api.github.com
	Scope     string
	now       func() time.Time
}

// DevicePending is the state between signIn and a successful signInPoll.
type DevicePending struct {
	ClientID   string
	DeviceCode string
	Interval   time.Duration
	Expires    time.Time
	LastPoll   time.Time
}

// PollOutcome is one poll's result: either still pending or a token.
type PollOutcome struct {
	Done  bool
	Token string
	User  string
}

func (d *DeviceFlow) clock() time.Time {
	if d.now != nil {
		return d.now()
	}
	return time.Now()
}

func (d *DeviceFlow) postForm(ctx context.Context, u string, form url.Values, out any) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, u, strings.NewReader(form.Encode()))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("Accept", "application/json")
	resp, err := d.HTTP.Do(req)
	if err != nil {
		return Errorf(CodeUpstream, "GitHub could not be reached: %v", err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return Errorf(CodeUpstream, "reading the GitHub reply failed: %v", err)
	}
	if resp.StatusCode/100 != 2 {
		return Errorf(CodeUpstream, "GitHub replied %d: %s", resp.StatusCode, strings.TrimSpace(string(body)))
	}
	if err := json.Unmarshal(body, out); err != nil {
		return Errorf(CodeUpstream, "GitHub sent an unreadable reply: %v", err)
	}
	return nil
}

// Start requests a device code.
func (d *DeviceFlow) Start(ctx context.Context, clientID string) (*DevicePending, *SignInResult, error) {
	var r struct {
		DeviceCode      string `json:"device_code"`
		UserCode        string `json:"user_code"`
		VerificationURI string `json:"verification_uri"`
		ExpiresIn       int    `json:"expires_in"`
		Interval        int    `json:"interval"`
		Error           string `json:"error"`
		ErrorDesc       string `json:"error_description"`
	}
	form := url.Values{"client_id": {clientID}}
	if d.Scope != "" {
		form.Set("scope", d.Scope)
	}
	if err := d.postForm(ctx, d.GitHubURL+"/login/device/code", form, &r); err != nil {
		return nil, nil, err
	}
	if r.Error != "" {
		return nil, nil, Errorf(CodeUpstream, "GitHub refused the device-flow request: %s %s", r.Error, r.ErrorDesc)
	}
	if r.DeviceCode == "" || r.UserCode == "" {
		return nil, nil, Errorf(CodeUpstream, "GitHub sent no device code")
	}
	if r.Interval <= 0 {
		r.Interval = 5
	}
	now := d.clock()
	p := &DevicePending{
		ClientID:   clientID,
		DeviceCode: r.DeviceCode,
		Interval:   time.Duration(r.Interval) * time.Second,
		Expires:    now.Add(time.Duration(r.ExpiresIn) * time.Second),
	}
	return p, &SignInResult{UserCode: r.UserCode, VerificationURI: r.VerificationURI, ExpiresIn: r.ExpiresIn}, nil
}

// Poll makes at most one token request. It returns Done=false while the user has not yet
// approved, or when called sooner than the interval GitHub asked for.
func (d *DeviceFlow) Poll(ctx context.Context, p *DevicePending) (PollOutcome, error) {
	now := d.clock()
	if now.After(p.Expires) {
		return PollOutcome{}, Errorf(CodeSignInExpired, "the sign-in code expired; start sign-in again")
	}
	if !p.LastPoll.IsZero() && now.Sub(p.LastPoll) < p.Interval {
		return PollOutcome{}, nil
	}
	p.LastPoll = now
	var r struct {
		AccessToken string `json:"access_token"`
		Error       string `json:"error"`
		ErrorDesc   string `json:"error_description"`
		Interval    int    `json:"interval"`
	}
	form := url.Values{
		"client_id":   {p.ClientID},
		"device_code": {p.DeviceCode},
		"grant_type":  {"urn:ietf:params:oauth:grant-type:device_code"},
	}
	if err := d.postForm(ctx, d.GitHubURL+"/login/oauth/access_token", form, &r); err != nil {
		return PollOutcome{}, err
	}
	switch r.Error {
	case "":
	case "authorization_pending":
		return PollOutcome{}, nil
	case "slow_down":
		if r.Interval > 0 {
			p.Interval = time.Duration(r.Interval) * time.Second
		} else {
			p.Interval += 5 * time.Second
		}
		return PollOutcome{}, nil
	case "expired_token":
		return PollOutcome{}, Errorf(CodeSignInExpired, "the sign-in code expired; start sign-in again")
	case "access_denied":
		return PollOutcome{}, Errorf(CodeSignInDenied, "sign-in was denied on GitHub")
	default:
		return PollOutcome{}, Errorf(CodeUpstream, "GitHub sign-in failed: %s %s", r.Error, r.ErrorDesc)
	}
	if r.AccessToken == "" {
		return PollOutcome{}, Errorf(CodeUpstream, "GitHub sent no access token")
	}
	user, err := d.user(ctx, r.AccessToken)
	if err != nil {
		return PollOutcome{}, err
	}
	return PollOutcome{Done: true, Token: r.AccessToken, User: user}, nil
}

func (d *DeviceFlow) user(ctx context.Context, token string) (string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, d.APIURL+"/user", nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Accept", "application/vnd.github+json")
	resp, err := d.HTTP.Do(req)
	if err != nil {
		return "", Errorf(CodeUpstream, "GitHub could not be reached: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 {
		return "", Errorf(CodeUpstream, "GitHub replied %d to the user lookup", resp.StatusCode)
	}
	var u struct {
		Login string `json:"login"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<20)).Decode(&u); err != nil {
		return "", Errorf(CodeUpstream, "GitHub sent an unreadable user: %v", err)
	}
	if u.Login == "" {
		return "", fmt.Errorf("GitHub sent no login")
	}
	return u.Login, nil
}
