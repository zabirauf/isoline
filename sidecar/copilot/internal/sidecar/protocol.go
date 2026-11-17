// Package sidecar implements the Isoline Copilot sidecar: newline-delimited JSON-RPC 2.0 on
// stdin and stdout, as specified in CONTRACTS.md section 6. Logs go to stderr only.
package sidecar

import (
	"encoding/json"
	"fmt"
)

// Version is reported by the status method.
const Version = "0.1.0"

// JSON-RPC error codes. The standard ones come first; the rest are Isoline's own.
const (
	CodeParseError     = -32700
	CodeInvalidRequest = -32600
	CodeMethodNotFound = -32601
	CodeInvalidParams  = -32602
	CodeInternal       = -32603

	// CodeNotSignedIn: a model call or model list needs a signed-in user.
	CodeNotSignedIn = -32001
	// CodeNoClientID: sign-in needs a GitHub OAuth app client id (a human item).
	CodeNoClientID = -32002
	// CodeLiveUnavailable: this binary was built without the Copilot SDK.
	CodeLiveUnavailable = -32003
	// CodeSignInExpired: the device code expired before the user approved it.
	CodeSignInExpired = -32004
	// CodeSignInDenied: the user denied the device-flow request.
	CodeSignInDenied = -32005
	// CodeNoPendingSignIn: signInPoll was called without a signIn first.
	CodeNoPendingSignIn = -32006
	// CodeUpstream: GitHub or the model returned an error.
	CodeUpstream = -32010
	// CodeKeychain: the OS keychain refused to store or read the token.
	CodeKeychain = -32011
)

// RPCError is a JSON-RPC error object. It also implements error.
type RPCError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    any    `json:"data,omitempty"`
}

func (e *RPCError) Error() string { return fmt.Sprintf("%d: %s", e.Code, e.Message) }

// Errorf builds an RPCError.
func Errorf(code int, format string, args ...any) *RPCError {
	return &RPCError{Code: code, Message: fmt.Sprintf(format, args...)}
}

type rpcRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params,omitempty"`
}

type rpcResponse struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Result  any             `json:"result,omitempty"`
	Error   *RPCError       `json:"error,omitempty"`
}

// ModelInfo mirrors iso_ai::ModelInfo. Prices are USD per million tokens.
type ModelInfo struct {
	ID              string  `json:"id"`
	Name            string  `json:"name"`
	InputPerM       float64 `json:"inputPerM"`
	OutputPerM      float64 `json:"outputPerM"`
	CachedInputPerM float64 `json:"cachedInputPerM"`
}

// CompletionRequest mirrors iso_ai::CompletionRequest.
type CompletionRequest struct {
	Model     string `json:"model"`
	System    string `json:"system"`
	Prompt    string `json:"prompt"`
	MaxTokens uint32 `json:"maxTokens"`
}

// Completion mirrors iso_ai::Completion. Every reply that used a model carries token counts.
type Completion struct {
	Text           string `json:"text"`
	Model          string `json:"model"`
	TokensIn       uint64 `json:"tokensIn"`
	TokensOut      uint64 `json:"tokensOut"`
	CachedTokensIn uint64 `json:"cachedTokensIn"`
}

// StatusResult is the result of status. Live is an addition: whether the Copilot SDK
// backend was compiled into this binary.
type StatusResult struct {
	SignedIn bool    `json:"signedIn"`
	User     *string `json:"user,omitempty"`
	Mock     bool    `json:"mock"`
	Version  string  `json:"version"`
	Live     bool    `json:"live"`
}

// SignInParams is the params object of signIn.
type SignInParams struct {
	ClientID string `json:"clientId,omitempty"`
}

// SignInResult is the result of signIn: the device-flow code to show the user.
type SignInResult struct {
	UserCode        string `json:"userCode"`
	VerificationURI string `json:"verificationUri"`
	ExpiresIn       int    `json:"expiresIn"`
}

// SignInPollResult is the result of signInPoll and signOut.
type SignInPollResult struct {
	SignedIn bool    `json:"signedIn"`
	User     *string `json:"user,omitempty"`
}

// ListModelsResult is the result of listModels.
type ListModelsResult struct {
	Models []ModelInfo `json:"models"`
}
