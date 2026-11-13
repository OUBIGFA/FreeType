package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"
)

// Serve the real GitHub API shapes; version selection, validation and file
// replacement still go through the production update library.
func updateFeed(t *testing.T, payload []byte, digest string) *httptest.Server {
	t.Helper()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/repos/OUBIGFA/FreeType/releases":
			json.NewEncoder(w).Encode([]any{
				map[string]any{"id": 9, "tag_name": "freetype-9.0.0", "draft": true, "assets": []any{map[string]any{"id": 9, "name": "FreeType-9.0.0-Setup.exe", "size": len(payload)}}},
				map[string]any{"id": 8, "tag_name": "freetype-8.0.0", "prerelease": true, "assets": []any{map[string]any{"id": 8, "name": "FreeType-8.0.0-Setup.exe", "size": len(payload)}}},
				map[string]any{"id": 7, "tag_name": "v7.0.0", "assets": []any{map[string]any{"id": 7, "name": "FreeType-7.0.0-Setup.exe", "size": len(payload)}}},
				map[string]any{"id": 1, "tag_name": "freetype-0.2.0", "body": "更新说明", "assets": []any{map[string]any{"id": 1, "name": "FreeType-0.2.0-Setup.exe", "size": len(payload), "digest": digest}}},
			})
		case "/repos/OUBIGFA/FreeType/releases/assets/1":
			if r.Header.Get("Accept") == "application/octet-stream" {
				w.Header().Set("Content-Type", "application/octet-stream")
				w.Write(payload)
			} else {
				json.NewEncoder(w).Encode(map[string]any{"id": 1, "digest": digest})
			}
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)
	return server
}

func TestUpdatesUseOnlyFreeTypeStableInstallerAndVerifyDownload(t *testing.T) {
	payload := []byte("MZ disposable installer bytes; never executed")
	sum := sha256.Sum256(payload)
	server := updateFeed(t, payload, "sha256:"+hex.EncodeToString(sum[:]))
	manager, err := newUpdates(t.TempDir(), "0.0.1", server.Client(), server.URL+"/")
	if err != nil {
		t.Fatal(err)
	}
	if err := manager.check(context.Background()); err != nil {
		t.Fatal(err)
	}
	if state := manager.snapshot(); state.Version != "0.2.0" || state.Ready || state.Message == "已是最新版本" {
		t.Fatalf("应选正式的 FreeType 安装器：%+v", state)
	}
	if err := manager.download(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !manager.snapshot().Ready {
		t.Fatal("校验通过的安装器应等待下一次启动")
	}
	data, err := os.ReadFile(manager.installerPath())
	if err != nil || string(data) != string(payload) {
		t.Fatalf("安装器内容不一致：%v", err)
	}
	if _, err := os.Stat(manager.pendingPath()); err != nil {
		t.Fatalf("待更新记录必须先保存：%v", err)
	}
	if err := manager.verifyPending(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(manager.installerPath(), []byte("changed after download"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := manager.verifyPending(); err == nil {
		t.Fatal("下次启动也必须拒绝已损坏的安装器")
	}
}

func TestFailedUpdateValidationDoesNotCreatePendingInstall(t *testing.T) {
	for _, digest := range []string{"", "sha256:" + fmt.Sprintf("%064d", 1)} {
		t.Run(digest, func(t *testing.T) {
			server := updateFeed(t, []byte("MZ invalid checksum fixture"), digest)
			manager, err := newUpdates(t.TempDir(), "0.0.1", server.Client(), server.URL+"/")
			if err != nil {
				t.Fatal(err)
			}
			if err := manager.check(context.Background()); err != nil {
				t.Fatal(err)
			}
			if err := manager.download(context.Background()); err == nil {
				t.Fatal("缺少或错误的校验值必须阻止更新")
			}
			if manager.snapshot().Ready {
				t.Fatal("校验失败不能显示更新已就绪")
			}
			if _, err := os.Stat(manager.pendingPath()); !os.IsNotExist(err) {
				t.Fatalf("校验失败不能写入待安装记录：%v", err)
			}
		})
	}
}

func TestPendingUpdateRejectsInvalidRecordAndDefaultsOff(t *testing.T) {
	dir := t.TempDir()
	manager, err := newUpdates(dir, "0.0.1", http.DefaultClient, "")
	if err != nil {
		t.Fatal(err)
	}
	if defaultConfig().AutoUpdate {
		t.Fatal("自动更新必须默认关闭")
	}
	if err := os.MkdirAll(filepath.Dir(manager.pendingPath()), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(manager.pendingPath(), []byte(`{"version":"../../unrelated","sha256":"bad"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := manager.verifyPending(); err == nil {
		t.Fatal("无效的本地待更新记录必须拒绝")
	}
}

func TestAutomaticUpdatesDownloadWithoutLaunchingAndSurviveRestart(t *testing.T) {
	payload := []byte("MZ automatic update fixture")
	sum := sha256.Sum256(payload)
	server := updateFeed(t, payload, "sha256:"+hex.EncodeToString(sum[:]))
	directory := t.TempDir()
	manager, err := newUpdates(directory, "0.0.1", server.Client(), server.URL+"/")
	if err != nil {
		t.Fatal(err)
	}
	var launches atomic.Int32
	manager.launch = func(string, string) error { launches.Add(1); return nil }
	ready := make(chan struct{}, 1)
	manager.changed = func(state updateState) {
		if state.Ready {
			select {
			case ready <- struct{}{}:
			default:
			}
		}
	}
	manager.setAutomatic(true)
	defer manager.close()
	select {
	case <-ready:
	case <-time.After(10 * time.Second):
		t.Fatal("开启自动更新后应后台下载并保存待安装记录")
	}
	manager.setAutomatic(false)
	if launches.Load() != 0 {
		t.Fatal("后台下载不能立即启动安装器")
	}
	restarted, err := newUpdates(directory, "0.0.1", server.Client(), server.URL+"/")
	if err != nil || !restarted.snapshot().Ready {
		t.Fatalf("下次启动必须读取已校验的下载：%v", err)
	}
	// An interrupted installation may have copied the new app before the
	// administrator cancels TSF registration. The same installer must retry.
	partiallyInstalled, err := newUpdates(directory, "0.2.0", server.Client(), server.URL+"/")
	if err != nil || !partiallyInstalled.snapshot().Ready {
		t.Fatalf("组件注册未完成时，同版本的待安装记录必须可重试：%v", err)
	}
	newerInstalled, err := newUpdates(directory, "0.3.0", server.Client(), server.URL+"/")
	if err != nil || newerInstalled.snapshot().Ready {
		t.Fatalf("更旧的待安装器不能用于降级：%v", err)
	}
	restarted.launch = func(path, dir string) error {
		if path != restarted.installerPath() || dir != `C:\Apps\FreeType` {
			t.Fatalf("应启动完整安装器并保留安装目录：%q %q", path, dir)
		}
		launches.Add(1)
		return nil
	}
	if err := restarted.install(`C:\Apps\FreeType`); err != nil || launches.Load() != 1 {
		t.Fatalf("校验成功后应进入完整安装流程：%v", err)
	}
	if err := restarted.install(`C:\Apps\FreeType`); err == nil || launches.Load() != 1 {
		t.Fatal("重复点击不能启动第二个安装器")
	}
}

func TestUpdateNowRedownloadsCorruptCacheAfterRestart(t *testing.T) {
	payload := []byte("MZ cached update fixture")
	sum := sha256.Sum256(payload)
	server := updateFeed(t, payload, "sha256:"+hex.EncodeToString(sum[:]))
	directory := t.TempDir()
	manager, err := newUpdates(directory, "0.0.1", server.Client(), server.URL+"/")
	if err != nil {
		t.Fatal(err)
	}
	if err := manager.check(context.Background()); err != nil {
		t.Fatal(err)
	}
	if err := manager.download(context.Background()); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(manager.installerPath(), []byte("damaged cache"), 0o600); err != nil {
		t.Fatal(err)
	}
	restarted, err := newUpdates(directory, "0.0.1", server.Client(), server.URL+"/")
	if err != nil {
		t.Fatal(err)
	}
	launched := false
	restarted.launch = func(string, string) error { launched = true; return nil }
	if err := restarted.updateNow(`C:\Apps\FreeType`); err != nil || !launched {
		t.Fatalf("立即更新应重新检查和下载损坏的缓存：%v", err)
	}
	if err := restarted.verifyPending(); err != nil {
		t.Fatal(err)
	}
}

func TestEmptyFeedAndApiFailureDoNotReportUpToDate(t *testing.T) {
	for _, status := range []int{http.StatusOK, http.StatusForbidden} {
		t.Run(fmt.Sprint(status), func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(status)
				if status == http.StatusOK {
					w.Write([]byte("[]"))
				} else {
					w.Write([]byte(`{"message":"rate limit"}`))
				}
			}))
			defer server.Close()
			manager, err := newUpdates(t.TempDir(), "0.0.1", server.Client(), server.URL+"/")
			if err != nil {
				t.Fatal(err)
			}
			err = manager.check(context.Background())
			if (err != nil) != (status != http.StatusOK) {
				t.Fatalf("必须区分尚无发布和检查失败：%v", err)
			}
			if state := manager.snapshot(); state.Message == "已是最新版本" || state.available() || state.Busy {
				t.Fatalf("缺少发布或请求失败不能报告已是最新：%+v", state)
			}
		})
	}
}
