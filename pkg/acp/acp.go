package acp

import (
	"bufio"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/mattn/go-isatty"
	"golang.org/x/sys/windows"
	"golang.org/x/text/encoding/japanese"
	"golang.org/x/text/transform"
)

// ErrWriter returns a writer that automatically converts UTF-8 to CP932 on Windows terminals
func ErrWriter() io.Writer {
	if runtime.GOOS != "windows" {
		return os.Stderr
	}

	// Detect console output code page.
	// 932 is the standard code page for Japanese Windows (Shift-S/CP932).
	cp, _ := windows.GetConsoleOutputCP()
	if cp == 932 {
		return transform.NewWriter(os.Stderr, japanese.ShiftJIS.NewEncoder())
	}

	if isatty.IsTerminal(os.Stderr.Fd()) || isatty.IsCygwinTerminal(os.Stderr.Fd()) {
		return transform.NewWriter(os.Stderr, japanese.ShiftJIS.NewEncoder())
	}
	return os.Stderr
}

// AcpError represents a JSON-RPC error from the agent.
type AcpError struct {
	Message string
}

func (e *AcpError) Error() string {
	return e.Message
}

// Request represents a JSON-RPC 2.0 request.
type Request struct {
	JsonRPC string      `json:"jsonrpc"`
	ID      uint64      `json:"id"`
	Method  string      `json:"method"`
	Params  interface{} `json:"params"`
}

// Response represents a JSON-RPC 2.0 response.
type Response struct {
	JsonRPC string          `json:"jsonrpc"`
	ID      uint64          `json:"id,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   *ErrorDetail    `json:"error,omitempty"`
}

// ErrorDetail represents JSON-RPC error details.
type ErrorDetail struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

// Notification represents a JSON-RPC 2.0 notification.
type Notification struct {
	JsonRPC string          `json:"jsonrpc"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params"`
}

// BuildRequest builds a JSON-RPC 2.0 request message.
func BuildRequest(id uint64, method string, params interface{}) string {
	req := Request{
		JsonRPC: "2.0",
		ID:      id,
		Method:  method,
		Params:  params,
	}
	data, _ := json.Marshal(req)
	return string(data)
}

// BuildInitializeParams builds the initialize request params.
func BuildInitializeParams() interface{} {
	return map[string]interface{}{
		"protocolVersion": 1,
		"clientInfo": map[string]string{
			"name":    "resident-ai-go",
			"version": "0.1.0",
		},
	}
}

// BuildSessionNewParams builds the session/new request params.
func BuildSessionNewParams(cwd string) interface{} {
	return map[string]interface{}{
		"cwd":        cwd,
		"mcpServers": []string{},
	}
}

// BuildPromptParams builds the session/prompt request params.
func BuildPromptParams(sessionID string, text string) interface{} {
	return map[string]interface{}{
		"sessionId": sessionID,
		"prompt": []map[string]string{
			{"type": "text", "text": text},
		},
	}
}

// BuildPromptText builds prompt text with @file references prepended.
func BuildPromptText(text string, files []string) string {
	if len(files) == 0 {
		return text
	}
	var sb strings.Builder
	for _, f := range files {
		sb.WriteString("@")
		sb.WriteString(f)
		sb.WriteString(" ")
	}
	sb.WriteString(text)
	return sb.String()
}

// Session represents an active ACP session with a running gemini process.
type Session struct {
	cmd       *exec.Cmd
	stdin     io.WriteCloser
	stdout    io.ReadCloser
	reader    *bufio.Reader
	sessionID string
	nextID    uint64
	model     string
}

// NewSession spawns gemini.cmd --acp, performs handshake, returns ready session.
func NewSession(cwd string) (*Session, error) {
	return NewSessionWithModel(cwd, "")
}

// NewSessionWithModel spawns gemini.cmd --acp with a specific model.
func NewSessionWithModel(cwd string, model string) (*Session, error) {
	args := []string{"--acp"}
	if model != "" {
		args = append(args, "-m", model)
	}

	cmd := exec.Command("gemini.cmd", args...)
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, fmt.Errorf("stdin pipe: %w", err)
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, fmt.Errorf("stdout pipe: %w", err)
	}
	cmd.Stderr = os.Stderr // Optional: redirect stderr for debugging

	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("spawn gemini.cmd --acp: %w", err)
	}

	s := &Session{
		cmd:    cmd,
		stdin:  stdin,
		stdout: stdout,
		reader: bufio.NewReader(stdout),
		nextID: 1,
		model:  model,
	}

	// initialize
	id, err := s.send("initialize", BuildInitializeParams())
	if err != nil {
		s.Close()
		return nil, err
	}
	resp, err := s.readUntilID(id)
	if err != nil {
		s.Close()
		return nil, err
	}
	if resp.Error != nil {
		s.Close()
		return nil, &AcpError{Message: fmt.Sprintf("initialize failed: %s", resp.Error.Message)}
	}

	// session/new
	id, err = s.send("session/new", BuildSessionNewParams(cwd))
	if err != nil {
		s.Close()
		return nil, err
	}
	resp, err = s.readUntilID(id)
	if err != nil {
		s.Close()
		return nil, err
	}
	if resp.Error != nil {
		s.Close()
		return nil, &AcpError{Message: fmt.Sprintf("session/new failed: %s", resp.Error.Message)}
	}

	var result struct {
		SessionID string `json:"sessionId"`
	}
	if err := json.Unmarshal(resp.Result, &result); err != nil {
		s.Close()
		return nil, fmt.Errorf("parse sessionId: %w", err)
	}
	s.sessionID = result.SessionID

	return s, nil
}

// Prompt sends a text prompt. Returns the full response text.
func (s *Session) Prompt(text string) (string, error) {
	params := BuildPromptParams(s.sessionID, text)
	id, err := s.send("session/prompt", params)
	if err != nil {
		return "", err
	}
	return s.collectResponse(id)
}

// PromptWithFiles sends a text prompt with @file references prepended.
func (s *Session) PromptWithFiles(text string, files []string) (string, error) {
	return s.Prompt(BuildPromptText(text, files))
}

// PromptWithImage sends a text prompt with an image embedded as base64.
func (s *Session) PromptWithImage(text string, imagePath string) (string, error) {
	return s.PromptWithImagesInline(text, []string{imagePath})
}

// PromptWithImagesInline sends a text prompt with N images embedded as base64.
func (s *Session) PromptWithImagesInline(text string, imagePaths []string) (string, error) {
	var promptItems []interface{}
	for _, path := range imagePaths {
		data, err := os.ReadFile(path)
		if err != nil {
			return "", fmt.Errorf("read image %s: %w", path, err)
		}
		b64 := base64.StdEncoding.EncodeToString(data)
		mime := getMimeType(path)
		promptItems = append(promptItems, map[string]string{
			"type":     "image",
			"mimeType": mime,
			"data":     b64,
		})
	}
	promptItems = append(promptItems, map[string]string{
		"type": "text",
		"text": text,
	})

	params := map[string]interface{}{
		"sessionId": s.sessionID,
		"prompt":    promptItems,
	}
	id, err := s.send("session/prompt", params)
	if err != nil {
		return "", err
	}
	return s.collectResponse(id)
}

func (s *Session) send(method string, params interface{}) (uint64, error) {
	id := s.nextID
	s.nextID++
	line := BuildRequest(id, method, params)
	
	// Truncate base64 for cleaner log
	logLine := line
	if len(logLine) > 1000 {
		logLine = logLine[:1000] + "...(truncated)"
	}
	fmt.Fprintf(ErrWriter(), "ACP SEND: %s\n", logLine)
	
	_, err := s.stdin.Write([]byte(line + "\n"))
	if err != nil {
		return 0, fmt.Errorf("write stdin: %w", err)
	}
	return id, nil
}

func (s *Session) readUntilID(expectedID uint64) (*Response, error) {
	for {
		line, err := s.readLine()
		if err != nil {
			return nil, err
		}
		var resp Response
		if err := json.Unmarshal(line, &resp); err == nil && resp.ID == expectedID {
			return &resp, nil
		}
	}
}

func (s *Session) collectResponse(expectedID uint64) (string, error) {
	var chunks []string
	for {
		line, err := s.readLine()
		if err != nil {
			return "", err
		}

		// Check if it's a response
		var resp Response
		if err := json.Unmarshal(line, &resp); err == nil && resp.ID == expectedID {
			if resp.Error != nil {
				return "", &AcpError{Message: fmt.Sprintf("prompt error: %s", resp.Error.Message)}
			}
			return strings.TrimSpace(strings.Join(chunks, "")), nil
		}

		// Check if it's a notification (chunk)
		var notif Notification
		if err := json.Unmarshal(line, &notif); err == nil && notif.Method == "session/update" {
			var params struct {
				Update struct {
					SessionUpdate string `json:"sessionUpdate"`
					Content      struct {
						Text string `json:"text"`
					} `json:"content"`
				} `json:"update"`
			}
			if err := json.Unmarshal(notif.Params, &params); err == nil {
				if params.Update.SessionUpdate == "agent_message_chunk" {
					chunks = append(chunks, params.Update.Content.Text)
				}
			}
		}
	}
}

func (s *Session) readLine() ([]byte, error) {
	for {
		line, err := s.reader.ReadBytes('\n')
		if err != nil {
			return nil, fmt.Errorf("read stdout: %w", err)
		}
		trimmed := strings.TrimSpace(string(line))
		if trimmed == "" {
			continue
		}
		return []byte(trimmed), nil
	}
}

func (s *Session) Close() {
	if s.stdin != nil {
		s.stdin.Close()
	}
	if s.cmd != nil && s.cmd.Process != nil {
		s.cmd.Process.Kill()
		s.cmd.Wait()
	}
}

func (s *Session) SessionID() string {
	return s.sessionID
}

func getMimeType(path string) string {
	ext := strings.ToLower(filepath.Ext(path))
	switch ext {
	case ".png":
		return "image/png"
	case ".jpg", ".jpeg":
		return "image/jpeg"
	case ".gif":
		return "image/gif"
	case ".webp":
		return "image/webp"
	default:
		return "application/octet-stream"
	}
}
