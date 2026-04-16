package api

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/YuujiKamura/resident-agent/pkg/acp"
)

type Backend int

const (
	Gemini Backend = iota
	Claude
	Codex
)

type UsageMode int

const (
	PayPerUse UsageMode = iota
	TimeBasedQuota
	Resident
)

type OutputFormat int

const (
	Text OutputFormat = iota
	Json
)

type AnalyzeOptions struct {
	Model        string
	OutputFormat OutputFormat
	Backend      Backend
	UsageMode    UsageMode
}

func DefaultOptions() AnalyzeOptions {
	return AnalyzeOptions{
		Model:        "",
		OutputFormat: Text,
		Backend:      Gemini,
		UsageMode:    TimeBasedQuota,
	}
}

func (o AnalyzeOptions) WithJSON() AnalyzeOptions {
	o.OutputFormat = Json
	return o
}

func (o AnalyzeOptions) WithBackend(b Backend) AnalyzeOptions {
	o.Backend = b
	return o
}

func (o AnalyzeOptions) WithUsageMode(m UsageMode) AnalyzeOptions {
	o.UsageMode = m
	return o
}

func (o AnalyzeOptions) WithModel(m string) AnalyzeOptions {
	o.Model = m
	return o
}

// Analyze analyzes a single file with a prompt. 
// Even if multiple files are provided, ONLY the first one is processed.
func Analyze(prompt string, files []string, options AnalyzeOptions) (string, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("getwd: %w", err)
	}

	session, err := acp.NewSessionWithModel(cwd, options.Model)
	if err != nil {
		return "", err
	}
	defer session.Close()

	fullPrompt := prompt
	if options.OutputFormat == Json {
		fullPrompt = fmt.Sprintf("%s Respond with ONLY the JSON object.", prompt)
	}

	if len(files) == 0 {
		return session.Prompt(fullPrompt)
	}

	// Strictly limit to the first file to ensure stability.
	targetFile := files[0]
	if isImage(targetFile) {
		// Use inline base64 for reliable image analysis.
		return session.PromptWithImage(fullPrompt, targetFile)
	}

	// For other files, use absolute @file reference.
	abs, err := filepath.Abs(targetFile)
	if err != nil {
		abs = targetFile
	}
	return session.PromptWithFiles(fullPrompt, []string{abs})
}

// Prompt performs a prompt without files.
func Prompt(prompt string, options AnalyzeOptions) (string, error) {
	return Analyze(prompt, nil, options)
}

func isImage(path string) bool {
	ext := strings.ToLower(filepath.Ext(path))
	switch ext {
	case ".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp":
		return true
	default:
		return false
	}
}
