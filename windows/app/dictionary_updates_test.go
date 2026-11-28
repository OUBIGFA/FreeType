package main

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// fakeDataBackend 模拟服务进程的词库更新记录：只登记和返回状态，校验仍由被测代码完成。
type fakeDataBackend struct {
	mu     sync.Mutex
	status DictionaryUpdateStatus
	calls  []Request
}

func (f *fakeDataBackend) Call(r Request) (Response, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, r)
	switch r["op"] {
	case "stage_dictionary_update":
		f.status.Pending = r["version"].(string)
		f.status.PendingAutomatic = r["automatic"].(bool)
	case "cancel_pending_dictionary":
		if !r["automatic_only"].(bool) || f.status.PendingAutomatic {
			f.status.Pending, f.status.PendingAutomatic = "", false
		}
	}
	status := f.status
	return Response{OK: true, DictionaryUpdate: &status}, nil
}

func (f *fakeDataBackend) ops() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []string
	for _, call := range f.calls {
		out = append(out, call["op"].(string))
	}
	return out
}

type builtPackage struct {
	zip, manifest []byte
}

// buildPackage 生成与工作流相同结构的压缩包和独立清单；edit 可以在写入前改动包内文件。
func buildPackage(t *testing.T, version, format string, edit func(files map[string][]byte)) builtPackage {
	t.Helper()
	files := map[string][]byte{"lexicon.ftd": []byte("FTD2 compiled lexicon " + version), "english.txt": []byte("github\n"), "LICENSE": []byte("license")}
	records := map[string]dataFile{}
	for _, name := range dataFiles {
		sum := sha256.Sum256(files[name])
		records[name] = dataFile{Bytes: int64(len(files[name])), SHA256: hex.EncodeToString(sum[:])}
	}
	inner := dataPackage{Format: 1, Channel: dataChannel, Version: version, LexiconFormat: format, Files: records}
	files[dataManifestName], _ = json.Marshal(inner)
	if edit != nil {
		edit(files)
	}
	var archive bytes.Buffer
	writer := zip.NewWriter(&archive)
	for name, data := range files {
		w, err := writer.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		w.Write(data)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256(archive.Bytes())
	outer := map[string]any{"format": 1, "channel": dataChannel, "version": version, "lexiconFormat": format, "files": records,
		"archive": map[string]any{"name": "FreeType-Dictionaries-" + version + ".zip", "bytes": archive.Len(), "sha256": hex.EncodeToString(sum[:])}}
	manifest, _ := json.Marshal(outer)
	return builtPackage{zip: archive.Bytes(), manifest: manifest}
}

// dataFeed 按 GitHub Release 接口的形状返回词库版本；requests 记下列表请求次数。
func dataFeed(t *testing.T, requests *atomic.Int32, packages map[string]builtPackage) *httptest.Server {
	t.Helper()
	assets := map[string][]byte{}
	var releases []any
	releases = append(releases,
		map[string]any{"id": 90, "tag_name": "freetype-0.3.0", "assets": []any{map[string]any{"id": 900, "name": "FreeType-0.3.0-Setup.exe", "size": 3}}},
		map[string]any{"id": 91, "tag_name": "freetype-data-2099.01.01.1", "draft": true, "assets": []any{}})
	id := 100
	for version, pkg := range packages {
		zipName, jsonName := "FreeType-Dictionaries-"+version+".zip", "FreeType-Dictionaries-"+version+".json"
		sum := sha256.Sum256(pkg.zip)
		releases = append(releases, map[string]any{"id": id, "tag_name": dataTagPrefix + version, "assets": []any{
			map[string]any{"id": id + 1, "name": jsonName, "size": len(pkg.manifest)},
			map[string]any{"id": id + 2, "name": zipName, "size": len(pkg.zip), "digest": "sha256:" + hex.EncodeToString(sum[:])},
		}})
		assets[strconv.Itoa(id+1)] = pkg.manifest
		assets[strconv.Itoa(id+2)] = pkg.zip
		id += 10
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/repos/OUBIGFA/FreeType/releases" {
			requests.Add(1)
			w.Header().Set("Content-Type", "application/json")
			json.NewEncoder(w).Encode(releases)
			return
		}
		if data, ok := assets[strings.TrimPrefix(r.URL.Path, "/repos/OUBIGFA/FreeType/releases/assets/")]; ok && r.Header.Get("Accept") == "application/octet-stream" {
			w.Header().Set("Content-Type", "application/octet-stream")
			w.Write(data)
			return
		}
		http.NotFound(w, r)
	}))
	t.Cleanup(server.Close)
	return server
}

func newTestDataUpdates(t *testing.T, server *httptest.Server, backend *fakeDataBackend) *dataUpdates {
	t.Helper()
	manager, err := newDataUpdates(backend, server.Client(), server.URL+"/")
	if err != nil {
		t.Fatal(err)
	}
	return manager
}

func TestDictionaryCheckChoosesNewestCompatiblePackageAndInstallsIt(t *testing.T) {
	var requests atomic.Int32
	server := dataFeed(t, &requests, map[string]builtPackage{
		"2026.12.06.1": buildPackage(t, "2026.12.06.1", "FTD9", nil),
		"2026.11.29.1": buildPackage(t, "2026.11.29.1", "FTD2", nil),
		"2026.11.10.1": buildPackage(t, "2026.11.10.1", "FTD2", nil),
	})
	backend := &fakeDataBackend{status: DictionaryUpdateStatus{Directory: t.TempDir(), Bundled: "2026-11-14.1"}}
	manager := newTestDataUpdates(t, server, backend)
	if err := manager.check(context.Background()); err != nil {
		t.Fatal(err)
	}
	if state := manager.snapshot(); state.Available != "2026.11.29.1" {
		t.Fatalf("新格式的版本不能凭日期较新被旧程序加载：%+v", state)
	}
	if err := manager.downloadAvailable(context.Background()); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(backend.status.Directory, "2026.11.29.1")
	for _, name := range []string{"lexicon.ftd", "english.txt", "PACKAGE.json", "LICENSE"} {
		if _, err := os.Stat(filepath.Join(dir, name)); err != nil {
			t.Fatalf("缺少解包文件 %s：%v", name, err)
		}
	}
	if _, err := os.Stat(filepath.Join(dir, "package.zip")); !os.IsNotExist(err) {
		t.Fatal("压缩包解开后不保留")
	}
	if backend.status.Pending != "2026.11.29.1" || backend.status.PendingAutomatic {
		t.Fatalf("手动下载登记为待生效、且不算自动安排：%+v", backend.status)
	}
	if state := manager.snapshot(); !strings.Contains(state.Message, "下次服务启动时生效") {
		t.Fatalf("应提示生效时机：%+v", state)
	}
	if err := manager.check(context.Background()); err != nil || manager.snapshot().Available != "" {
		t.Fatalf("已安排生效的版本不再提示：%+v %v", manager.snapshot(), err)
	}
}

func TestDictionaryDownloadRejectsDamagedOrUnexpectedPackages(t *testing.T) {
	cases := map[string]func(files map[string][]byte){
		"内容与清单不符": func(files map[string][]byte) { files["lexicon.ftd"] = []byte("FTD2 compiled lexicon 2026.11.29.X") },
		"路径越界":    func(files map[string][]byte) { files["../evil.txt"] = []byte("x") },
		"意外文件":    func(files map[string][]byte) { files["run.exe"] = []byte("x") },
		"缺少清单":    func(files map[string][]byte) { delete(files, dataManifestName) },
	}
	for name, edit := range cases {
		t.Run(name, func(t *testing.T) {
			var requests atomic.Int32
			server := dataFeed(t, &requests, map[string]builtPackage{"2026.11.29.1": buildPackage(t, "2026.11.29.1", "FTD2", edit)})
			backend := &fakeDataBackend{status: DictionaryUpdateStatus{Directory: t.TempDir(), Bundled: "2026-11-14.1", Current: "2026.11.22.1"}}
			manager := newTestDataUpdates(t, server, backend)
			if err := manager.check(context.Background()); err != nil {
				t.Fatal(err)
			}
			if err := manager.downloadAvailable(context.Background()); err == nil {
				t.Fatal("损坏的更新包必须拒绝")
			}
			entries, _ := os.ReadDir(backend.status.Directory)
			if len(entries) != 0 || backend.status.Pending != "" {
				t.Fatalf("失败不能留下目录或待生效记录：%v %+v", entries, backend.status)
			}
		})
	}
}

func TestAutomaticDictionaryUpdatesCheckWeeklyAndSkipFailedVersions(t *testing.T) {
	var requests atomic.Int32
	server := dataFeed(t, &requests, map[string]builtPackage{"2026.11.29.1": buildPackage(t, "2026.11.29.1", "FTD2", nil)})
	backend := &fakeDataBackend{status: DictionaryUpdateStatus{Directory: t.TempDir(), Bundled: "2026-11-14.1", FailedVersion: "2026.11.29.1"}}
	manager := newTestDataUpdates(t, server, backend)
	now := time.Date(2026, 11, 30, 4, 0, 0, 0, time.UTC)
	manager.now = func() time.Time { return now }
	if err := manager.runAutomatic(context.Background()); err != nil {
		t.Fatal(err)
	}
	if backend.status.Pending != "" {
		t.Fatal("上次未能生效的版本不能自动重复安排")
	}
	if err := manager.runAutomatic(context.Background()); err != nil || requests.Load() != 1 {
		t.Fatalf("七天内不重复检查：%d %v", requests.Load(), err)
	}
	backend.status.FailedVersion = ""
	now = now.Add(8 * 24 * time.Hour)
	if err := manager.runAutomatic(context.Background()); err != nil {
		t.Fatal(err)
	}
	if requests.Load() != 2 || backend.status.Pending != "2026.11.29.1" || !backend.status.PendingAutomatic {
		t.Fatalf("满七天后检查并自动安排：%d %+v", requests.Load(), backend.status)
	}
}

func TestDisablingAutomaticDictionaryUpdatesCancelsOnlyAutomaticStaging(t *testing.T) {
	var requests atomic.Int32
	server := dataFeed(t, &requests, nil)
	backend := &fakeDataBackend{status: DictionaryUpdateStatus{Directory: t.TempDir(), Pending: "2026.11.29.1", PendingAutomatic: true}}
	manager := newTestDataUpdates(t, server, backend)
	manager.automatic = true
	manager.setAutomatic(false)
	deadline := time.Now().Add(5 * time.Second)
	for !strings.Contains(strings.Join(backend.ops(), ","), "cancel_pending_dictionary") {
		if time.Now().After(deadline) {
			t.Fatal("关闭自动更新应撤销自动安排")
		}
		time.Sleep(10 * time.Millisecond)
	}
	backend.mu.Lock()
	defer backend.mu.Unlock()
	if backend.status.Pending != "" {
		t.Fatalf("自动安排未撤销：%+v", backend.status)
	}
}

func TestDataVersionsCompareBundledAndPackageVersions(t *testing.T) {
	if !newerData("2026.11.29.1", "2026-11-14.1") || newerData("2026.11.14.1", "2026-11-14.1") {
		t.Fatal("安装包日期版本与更新包版本应可比较")
	}
	if newerData("bad", "2026.11.29.1") || !newerData("2026.11.29.2", "2026.11.29.1") {
		t.Fatal("版本比较错误")
	}
	status := DictionaryUpdateStatus{Bundled: "2026-11-14.1", Active: "2026.11.22.1", Pending: "2026.11.29.1"}
	if status.baseline() != "2026.11.29.1" {
		t.Fatalf("基准应为最新的已用或待生效版本：%s", status.baseline())
	}
}

// 词库页显示更新状态；自动更新默认关闭，打开后保存到设置并通知更新管理器。
func TestDictionaryPageShowsUpdateStatusAndSavesTheAutomaticSwitch(t *testing.T) {
	s, b := newTestSettings()
	s.refresh()
	s.dataUpdate = dataUpdateState{Loaded: true, Message: "词库已是最新",
		Status: DictionaryUpdateStatus{Bundled: "2026-11-14.1", Active: "2026.11.22.1", Current: "2026.11.22.1", Pending: "2026.11.29.1"}}
	tt := open(t, s, "词库")
	tt.Frame()
	if b.config.DictionaryAutoUpdate {
		t.Fatal("自动更新词库应默认关闭")
	}
	names := map[string]bool{}
	for _, control := range tt.AccessAll() {
		names[control.Name] = true
	}
	for _, label := range []string{"检查词库更新", "下载词库更新", "恢复上一版词库", "取消待生效的词库"} {
		if !names[label] {
			t.Fatalf("词库更新缺少操作：%s", label)
		}
	}
	if err := tt.Click("自动更新词库"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !b.config.DictionaryAutoUpdate {
		t.Fatal("自动更新词库没有保存")
	}
}
