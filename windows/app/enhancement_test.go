package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"github.com/egoist/mygo/ui"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestEnhancementDownloadRequiresCompleteVerifiedFile(t *testing.T) {
	data := []byte("test model bytes")
	sum := sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write(data) }))
	defer server.Close()
	path := filepath.Join(t.TempDir(), "nova.gguf")
	if err := downloadEnhancement(context.Background(), server.Client(), server.URL, path, digest, int64(len(data))); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(path); string(got) != string(data) {
		t.Fatal("downloaded bytes differ")
	}
	if err := downloadEnhancement(context.Background(), server.Client(), server.URL, path, digest, int64(len(data))); err == nil {
		t.Fatal("must not overwrite an installed model")
	}
	if got, _ := os.ReadFile(path); string(got) != string(data) {
		t.Fatal("existing model changed")
	}
}

func TestEnhancementControlsDistinguishDownloadAndEnable(t *testing.T) {
	for _, tc := range []struct {
		name                string
		downloaded, enabled bool
		button              string
	}{{"download", false, false, "下载约 116 MB"}, {"enable", true, false, "启用"}, {"active", true, true, "关闭"}} {
		t.Run(tc.name, func(t *testing.T) {
			s, _ := newTestSettings()
			s.enhancementChecked = time.Now()
			s.enhancementDownloaded = tc.downloaded
			s.enhancement = EnhancementStatus{Enabled: tc.enabled, State: "ready"}
			s.locateSetting(setting("features/Nova 增强模型"))
			v := ui.NewTester(s.view, 1100, 1400)
			v.Frame()
			if _, ok := v.Find(tc.button); !ok {
				t.Fatal("missing action", tc.button)
			}
			saveUIFrame(t, "enhancement-"+tc.name, v)
		})
	}
}

func TestEnhancementRejectsTruncatedCorruptAndCancelledDownloads(t *testing.T) {
	data := []byte("bytes")
	sum := sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write(data) }))
	defer server.Close()
	for _, tc := range []struct {
		name, digest string
		size         int64
		cancel       bool
	}{{"truncated", digest, 6, false}, {"corrupt", "bad", 5, false}, {"cancel", digest, 5, true}} {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			path := filepath.Join(dir, "model.gguf")
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			if tc.cancel {
				cancel()
			}
			if err := downloadEnhancement(ctx, server.Client(), server.URL, path, tc.digest, tc.size); err == nil {
				t.Fatal("invalid download accepted")
			}
			entries, _ := os.ReadDir(dir)
			if len(entries) != 0 {
				t.Fatal("incomplete model remained visible")
			}
		})
	}
}
