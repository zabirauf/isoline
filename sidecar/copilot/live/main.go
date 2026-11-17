// Command copilot-sidecar (live build) is the Isoline Copilot sidecar with the GitHub
// Copilot SDK linked in. It lives in its own module because the SDK needs Go 1.26, while
// the protocol, mock mode and tests in the parent module build with older toolchains.
package main

import (
	"os"

	"isoline/sidecar/copilot/internal/sidecar"
)

func main() {
	os.Exit(sidecar.Main(os.Args[1:], newCopilotBackend))
}
