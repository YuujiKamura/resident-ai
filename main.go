package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"

	"github.com/YuujiKamura/resident-agent/pkg/api"
)

type ClientRequest struct {
	Prompt string   `json:"prompt"`
	Images []string `json:"images"`
	Model  string   `json:"model"`
}

func main() {
	// Re-introduce model flag for direct prompt usage
	modelFlag := flag.String("m", "", "Model name for direct prompt usage")
	// Dummy flag for compatibility
	_ = flag.Bool("json", false, "JSON output mode (deprecated, always on)")
	flag.Parse()

	args := flag.Args()
	if len(args) == 0 {
		fmt.Fprintf(os.Stderr, "Error: expected a file path or a direct prompt\n")
		os.Exit(1)
	}

	var prompt, model string
	var images []string
	var opts api.AnalyzeOptions

	// Attempt to process first arg as a JSON request file
	firstArg := args[0]
	reqData, fileErr := os.ReadFile(firstArg)
	var req ClientRequest
	parsedAsFile := false
	if fileErr == nil {
		if json.Unmarshal(reqData, &req) == nil && req.Prompt != "" {
			prompt = req.Prompt
			images = req.Images
			model = req.Model
			parsedAsFile = true
		}
	}

	// If not a valid request file, treat as a direct prompt
	if !parsedAsFile {
		prompt = firstArg
		images = args[1:]
		model = *modelFlag
	}

	// Final check for prompt
	if prompt == "" {
		fmt.Fprintf(os.Stderr, "Error: prompt could not be determined\n")
		os.Exit(1)
	}

	// Always output JSON, and use the determined model.
	opts = api.DefaultOptions().WithModel(model).WithJSON()

	result, err := api.Analyze(prompt, images, opts)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	fmt.Println(result)
}
