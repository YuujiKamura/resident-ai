package acp

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"strings"
	"testing"
)

// MockAgent implements io.ReadWriter to simulate gemini.cmd --acp
type MockAgent struct {
	io.Reader
	io.Writer
	responses chan string
}

func (m *MockAgent) Write(p []byte) (n int, err error) {
	// Simulate agent processing requests
	var req Request
	if err := json.Unmarshal(p, &req); err == nil {
		switch req.Method {
		case "initialize":
			m.responses <- `{"jsonrpc":"2.0","id":` + fmt.Sprint(req.ID) + `,"result":{"protocolVersion":1}}`
		case "session/new":
			m.responses <- `{"jsonrpc":"2.0","id":` + fmt.Sprint(req.ID) + `,"result":{"sessionId":"mock-session-123"}}`
		case "session/prompt":
			// Simulate chunked response
			m.responses <- `{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"Hello "}}}}`
			m.responses <- `{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"World!"}}}}`
			m.responses <- `{"jsonrpc":"2.0","id":` + fmt.Sprint(req.ID) + `,"result":{}}`
		}
	}
	return len(p), nil
}

func (m *MockAgent) Read(p []byte) (n int, err error) {
	resp := <-m.responses
	return strings.NewReader(resp + "\n").Read(p)
}

func TestSessionWithMock(t *testing.T) {
	// Note: We need a way to inject MockAgent into Session.
	// Since NewSession currently calls exec.Command, let's test the logic parts.
}

func TestCollectResponse_Mock(t *testing.T) {
	responses := `{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"Hello "}}}}
{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"World!"}}}}
{"jsonrpc":"2.0","id":1,"result":{}}
`
	reader := bufio.NewReader(strings.NewReader(responses))
	s := &Session{
		reader: reader,
	}

	got, err := s.collectResponse(1)
	if err != nil {
		t.Fatalf("collectResponse failed: %v", err)
	}
	want := "Hello World!"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestCollectResponse_Error(t *testing.T) {
	responses := `{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"Internal Error"}}
`
	reader := bufio.NewReader(strings.NewReader(responses))
	s := &Session{
		reader: reader,
	}

	_, err := s.collectResponse(1)
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if !strings.Contains(err.Error(), "Internal Error") {
		t.Errorf("expected error message to contain 'Internal Error', got %v", err)
	}
}

func TestReadUntilID_SkipNotifications(t *testing.T) {
	responses := `{"jsonrpc":"2.0","method":"some/notification","params":{}}
{"jsonrpc":"2.0","id":5,"result":{"ok":true}}
`
	reader := bufio.NewReader(strings.NewReader(responses))
	s := &Session{
		reader: reader,
	}

	resp, err := s.readUntilID(5)
	if err != nil {
		t.Fatalf("readUntilID failed: %v", err)
	}
	if resp.ID != 5 {
		t.Errorf("expected ID 5, got %d", resp.ID)
	}
}
