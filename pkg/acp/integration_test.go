//go:build integration_gdrive

package acp

import (
	"os"
	"testing"
)

func TestSession_PromptWithImage_Integration_GDrive(t *testing.T) {
	cwd, _ := os.Getwd()
	session, err := NewSession(cwd)
	if err != nil {
		t.Fatalf("Failed to start session: %v", err)
	}
	defer session.Close()

	// ユーザー指定のGoogleドライブパスをテストに内挿
	testImage := "I:\\マイドライブ\\過去の現場_元請\\2025.3.17 東区市道（2工区）舗装補修工事（水防等含）（単価契約）\\20260331 画図町下無田 ※追加でせんばんと！？\\工事写真\\1.施工状況\\R0010851.JPG"
	
	if _, err := os.Stat(testImage); os.IsNotExist(err) {
		t.Skipf("Test image not found at specified path (skipping GDrive test): %s", testImage)
	}

	t.Run("GoogleDriveImageTest", func(t *testing.T) {
		resp, err := session.PromptWithImage("この画像の内容を説明しろ", testImage)
		if err != nil {
			t.Fatalf("Prompt failed: %v", err)
		}
		if resp == "" {
			t.Fatal("Response is empty")
		}
		t.Logf("Response: %s", resp)
	})
}
