package acp

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestSession_LongRunning(t *testing.T) {
	// gemini.cmd がパスにあるか確認（なければスキップ）
	if _, err := exec.LookPath("gemini.cmd"); err != nil {
		t.Skip("gemini.cmd not found in PATH, skipping long session test")
	}

	cwd, _ := os.Getwd()
	// pkg/acp から実行されるため、ルートの画像を参照
	testImage := filepath.Join(cwd, "..", "..", "R0010851.JPG")
	if _, err := os.Stat(testImage); os.IsNotExist(err) {
		t.Skipf("Local test image not found at %s", testImage)
	}

	session, err := NewSession(cwd)
	if err != nil {
		t.Fatalf("Failed to start session: %v", err)
	}
	defer session.Close()

	t.Logf("Started session: %s", session.SessionID())

	// 1回目: テキストのみ
	t.Run("Prompt1_Text", func(t *testing.T) {
		resp, err := session.Prompt("1+1=? 数字だけ答えろ")
		if err != nil {
			t.Fatalf("First prompt failed: %v", err)
		}
		t.Logf("Response 1: %s", resp)
		if resp == "" {
			t.Error("Response 1 is empty")
		}
	})

	// 2回目: 同一セッションで画像
	t.Run("Prompt2_Image", func(t *testing.T) {
		resp, err := session.PromptWithImage("この画像には何が写っていますか？簡潔に答えろ", testImage)
		if err != nil {
			t.Fatalf("Second prompt (image) failed: %v", err)
		}
		t.Logf("Response 2: %s", resp)
		if resp == "" {
			t.Error("Response 2 is empty")
		}
	})

	// 3回目: 再度テキストのみ
	t.Run("Prompt3_Text", func(t *testing.T) {
		resp, err := session.Prompt("さっきの画像に人は写っていましたか？")
		if err != nil {
			t.Fatalf("Third prompt failed: %v", err)
		}
		t.Logf("Response 3: %s", resp)
		if resp == "" {
			t.Error("Response 3 is empty")
		}
	})
}
