package acp

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestSession_PromptWithLocalImage_Integration(t *testing.T) {
	// gemini.cmd がパスにあるか確認（なければスキップ）
	if _, err := exec.LookPath("gemini.cmd"); err != nil {
		t.Skip("gemini.cmd not found in PATH, skipping integration test")
	}

	cwd, _ := os.Getwd()
	// テスト用画像は resident-agent ルートにある想定
	testImage := filepath.Join(cwd, "..", "..", "R0010851.JPG")
	
	if _, err := os.Stat(testImage); os.IsNotExist(err) {
		t.Skipf("Local test image not found at %s", testImage)
	}

	session, err := NewSession(cwd)
	if err != nil {
		t.Fatalf("Failed to start session: %v", err)
	}
	defer session.Close()

	t.Run("LocalImageTest", func(t *testing.T) {
		resp, err := session.PromptWithImage("この画像に何が写っていますか？", testImage)
		if err != nil {
			t.Fatalf("Prompt failed: %v", err)
		}
		if resp == "" {
			t.Fatal("Response is empty")
		}
		t.Logf("Response: %s", resp)
	})
}
