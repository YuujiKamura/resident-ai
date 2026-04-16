package api

import (
	"testing"
)

func TestDefaultOptions(t *testing.T) {
	opts := DefaultOptions()
	if opts.Backend != Gemini {
		t.Errorf("expected backend Gemini, got %v", opts.Backend)
	}
	if opts.UsageMode != TimeBasedQuota {
		t.Errorf("expected usageMode TimeBasedQuota, got %v", opts.UsageMode)
	}
	if opts.OutputFormat != Text {
		t.Errorf("expected outputFormat Text, got %v", opts.OutputFormat)
	}
}

func TestIsImage(t *testing.T) {
	tests := []struct {
		path string
		want bool
	}{
		{"photo.jpg", true},
		{"photo.png", true},
		{"doc.pdf", false},
		{"data.json", false},
	}

	for _, tt := range tests {
		if got := isImage(tt.path); got != tt.want {
			t.Errorf("isImage(%q) = %v; want %v", tt.path, got, tt.want)
		}
	}
}
