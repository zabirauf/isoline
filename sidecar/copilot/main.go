// Command copilot-sidecar is the mock-only build of the Isoline Copilot sidecar. It speaks
// the full protocol; live model calls need the build in ./live, which links the GitHub
// Copilot SDK (Go 1.26). build.sh picks the live build when it can.
package main

import (
	"os"

	"isoline/sidecar/copilot/internal/sidecar"
)

func main() {
	os.Exit(sidecar.Main(os.Args[1:], nil))
}
