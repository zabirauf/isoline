package server

import (
	"context"
	"fmt"
	nethttp "net/http"
	_ "embed"
)

// MaxConns limits connections.
const MaxConns = 10

var debug = false

// Server serves requests.
// It is safe for concurrent use.
type Server struct {
	addr string
}

type Handler interface {
	Serve(ctx context.Context) error
}

type ID string

// New makes a server.
func New(addr string) *Server {
	return &Server{addr: addr}
}

func (s *Server) Start(ctx context.Context) error {
	fmt.Println("listening on", s.addr)
	return nethttp.ListenAndServe(s.addr, nil)
}

func helper() {
	New("x").Start(nil)
}
