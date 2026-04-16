package acp

import (
	"encoding/json"
	"testing"
)

func TestBuildRequest(t *testing.T) {
	msg := BuildRequest(1, "initialize", BuildInitializeParams())
	var parsed map[string]interface{}
	if err := json.Unmarshal([]byte(msg), &parsed); err != nil {
		t.Fatalf("parse JSON: %v", err)
	}

	if parsed["jsonrpc"] != "2.0" {
		t.Errorf("expected jsonrpc 2.0, got %v", parsed["jsonrpc"])
	}
	if parsed["id"].(float64) != 1 {
		t.Errorf("expected id 1, got %v", parsed["id"])
	}
	if parsed["method"] != "initialize" {
		t.Errorf("expected method initialize, got %v", parsed["method"])
	}

	params := parsed["params"].(map[string]interface{})
	if params["protocolVersion"].(float64) != 1 {
		t.Errorf("expected protocolVersion 1, got %v", params["protocolVersion"])
	}
	clientInfo := params["clientInfo"].(map[string]interface{})
	if clientInfo["name"] != "resident-ai-go" {
		t.Errorf("expected name resident-ai-go, got %v", clientInfo["name"])
	}
}

func TestBuildPromptText(t *testing.T) {
	tests := []struct {
		text  string
		files []string
		want  string
	}{
		{"hello", nil, "hello"},
		{"hello", []string{}, "hello"},
		{"analyze", []string{"a.jpg", "b.pdf"}, "@a.jpg @b.pdf analyze"},
	}

	for _, tt := range tests {
		if got := BuildPromptText(tt.text, tt.files); got != tt.want {
			t.Errorf("BuildPromptText(%q, %v) = %q; want %q", tt.text, tt.files, got, tt.want)
		}
	}
}
