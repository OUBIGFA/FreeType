package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"image/png"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// 保存连接设置不能丢掉用户设定的后台同步计划。
func TestAutoSyncOptionsSurviveSavingConnection(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "sync.json")
	err := os.WriteFile(path, []byte(`{"enabled":true,"automatic":true,"schedule":"daily","daily_time":"21:30","interval_minutes":45,"daily_limit":8,"retry_limit":2,"keep_snapshots":7}`), 0600)
	if err != nil {
		t.Fatal(err)
	}
	c, err := loadSyncSettings(dir)
	if err != nil {
		t.Fatal(err)
	}
	if err := saveSyncSettings(dir, c); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var saved map[string]any
	if err := json.Unmarshal(data, &saved); err != nil {
		t.Fatal(err)
	}
	if saved["automatic"] != true || saved["daily_time"] != "21:30" || saved["daily_limit"] != float64(8) || saved["keep_snapshots"] != float64(7) {
		t.Fatalf("保存连接时丢失自动同步计划：%s", data)
	}
}

// 管道是外部进程边界；实际快照传输、版本决策与状态文件使用生产实现。
type syncBackendFixture struct {
	data     []byte
	modified int64
	imports  int
	fail     bool
}

// 单一记录的外部服务契约夹具；记录级合并算法由 Rust 的真实双设备测试覆盖。
func testRecordSnapshot(data []byte, modified int64) []byte {
	var value map[string]json.RawMessage
	_ = json.Unmarshal(data, &value)
	value["test_record_modified"], _ = json.Marshal(modified)
	encoded, _ := json.Marshal(value)
	return encoded
}

func testRecordContent(data []byte) ([]byte, int64) {
	var value map[string]json.RawMessage
	_ = json.Unmarshal(data, &value)
	var modified int64
	_ = json.Unmarshal(value["test_record_modified"], &modified)
	delete(value, "test_record_modified")
	encoded, _ := json.Marshal(value)
	return encoded, modified
}

func (b *syncBackendFixture) Call(r Request) (Response, error) {
	path, _ := r["path"].(string)
	switch r["op"] {
	case "export_sync_settings":
		return Response{OK: true, ModifiedUnixMS: b.modified, SyncFormat: 1}, os.WriteFile(path, testRecordSnapshot(b.data, b.modified), 0600)
	case "preview_sync_merge":
		data, err := os.ReadFile(path)
		if err != nil {
			return Response{}, err
		}
		_, modified := testRecordContent(data)
		preview := &SyncMergePreview{}
		if modified > b.modified {
			preview.Updated, preview.Versions = 1, 1
		}
		return Response{OK: true, SyncMerge: preview}, nil
	case "inspect_settings_backup":
		data, err := os.ReadFile(path)
		if err == nil && !json.Valid(data) {
			err = errors.New("无效备份")
		}
		return Response{OK: err == nil}, err
	case "merge_sync_settings":
		expected, err := os.ReadFile(r["expected_path"].(string))
		if err != nil {
			return Response{}, err
		}
		if !bytes.Equal(expected, testRecordSnapshot(b.data, b.modified)) || b.fail {
			return Response{}, errors.New("本机数据已改变")
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return Response{}, err
		}
		b.data, b.modified = testRecordContent(data)
		b.imports++
		return Response{OK: true, SyncMerge: &SyncMergePreview{Updated: 1, Versions: 1}}, nil
	}
	return Response{}, errors.New("意外的同步操作")
}

func TestAutoSyncTransfersNewestSnapshotOverWebDAVAndDoesNotEchoAfterRestart(t *testing.T) {
	dav := &fakeDAV{t: t, dirs: map[string]bool{"": true, "/dav": true}, files: map[string][]byte{}}
	server := httptest.NewServer(dav)
	defer server.Close()
	dir := t.TempDir()
	c, err := loadSyncSettings(dir)
	if err != nil {
		t.Fatal(err)
	}
	c.Enabled, c.Automatic, c.DeviceID = true, true, deviceA
	c.WebDAVURL, c.WebDAVUser, c.WebDAVPassword = server.URL+"/dav/FreeType", "me", "secret"
	if err = saveSyncSettings(dir, c); err != nil {
		t.Fatal(err)
	}
	b := &syncBackendFixture{data: []byte(`{"format":"freetype-settings","value":"local"}`), modified: 100}
	a, err := newAutoSync(dir, b)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = a.cycle(c); err != nil {
		t.Fatal(err)
	}
	store, err := a.openStore(c)
	if err != nil {
		t.Fatal(err)
	}
	devices, err := listDevices(store)
	if err != nil || len(devices) != 1 {
		t.Fatalf("%+v %v", devices, err)
	}
	if devices[0].Modified != 100 {
		t.Fatal(devices)
	}
	remote := filepath.Join(t.TempDir(), "remote.json")
	if err = os.WriteFile(remote, testRecordSnapshot([]byte(`{"format":"freetype-settings","value":"remote"}`), 200), 0600); err != nil {
		t.Fatal(err)
	}
	hash, _ := snapshotHash(remote)
	if _, err = uploadVersionedSnapshot(store, deviceB, "另一台", remote, time.Now(), syncVersion{Modified: 200, Origin: deviceB, Hash: hash}, 5); err != nil {
		t.Fatal(err)
	}
	if _, err = a.cycle(c); err != nil {
		t.Fatal(err)
	}
	if b.imports != 1 || !bytes.Contains(b.data, []byte("remote")) {
		t.Fatalf("没有下载最新版本：%s", b.data)
	}
	backups, _ := filepath.Glob(filepath.Join(dir, "sync-backups", "*.json"))
	if len(backups) != 1 {
		t.Fatal("缺少恢复前备份", backups)
	}
	old, _ := os.ReadFile(backups[0])
	if !bytes.Contains(old, []byte("local")) {
		t.Fatal(string(old))
	}
	before := len(dav.files)
	restarted, err := newAutoSync(dir, b)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = restarted.cycle(c); err != nil {
		t.Fatal(err)
	}
	if len(dav.files) != before || b.imports != 1 {
		t.Fatal("重启后把下载内容当新版本回传了")
	}
	b.data = []byte(`{"format":"freetype-settings","value":"edited"}`)
	b.modified = 300
	if _, err = restarted.cycle(c); err != nil {
		t.Fatal(err)
	}
	devices, _ = listDevices(store)
	if newestSyncDevice(devices).Modified != 300 {
		t.Fatal("新编辑没有上传", devices)
	}
}

func TestAutoSyncRetriesHonorsDailyLimitAndPersistsAcrossRestart(t *testing.T) {
	dir := t.TempDir()
	c, _ := loadSyncSettings(dir)
	c.Enabled, c.Automatic, c.DeviceID = true, true, deviceA
	c.DailyLimit = 2
	if err := saveSyncSettings(dir, c); err != nil {
		t.Fatal(err)
	}
	b := &syncBackendFixture{data: []byte(`{}`), modified: 100}
	a, err := newAutoSync(dir, b)
	if err != nil {
		t.Fatal(err)
	}
	attempts := 0
	a.openStore = func(syncSettings) (remoteStore, error) { attempts++; return nil, errors.New("网络不可用") }
	now := time.Date(2026, 11, 17, 10, 0, 0, 0, time.Local)
	a.state.Next = now
	a.run(now, false)
	if attempts != 1 || a.state.Next.Before(now.Add(time.Minute)) || a.state.Next.After(now.Add(2*time.Minute)) {
		t.Fatal("首次失败应一分钟后重试", a.state)
	}
	a.run(now.Add(59*time.Second), false)
	if attempts != 1 {
		t.Fatal("提前重试")
	}
	a.run(a.state.Next, false)
	if attempts != 2 {
		t.Fatal(attempts)
	}
	a.run(a.state.Next, false)
	if attempts != 2 || a.state.Count != 2 {
		t.Fatal("超过每日上限")
	}
	restarted, err := newAutoSync(dir, b)
	if err != nil {
		t.Fatal(err)
	}
	restarted.openStore = a.openStore
	restarted.state.Next = now
	restarted.run(now.Add(4*time.Minute), false)
	if attempts != 2 {
		t.Fatal("重启绕过每日限制")
	}
	restarted.config.Automatic = false
	restarted.run(now.AddDate(0, 0, 1), false)
	if attempts != 2 {
		t.Fatal("关闭后仍自动运行")
	}
	restarted.config.Automatic = true
	restarted.state.Next = now
	restarted.run(now.AddDate(0, 0, 1), false)
	if attempts != 3 || restarted.state.Count != 1 {
		t.Fatal("次日未恢复配额")
	}
}

func TestAutoSyncChoosesNewestDataNotNewestUpload(t *testing.T) {
	local := syncVersion{Modified: 200, Origin: deviceA, Hash: "local"}
	older := syncDevice{ID: deviceB, Revision: "900-1", Uploaded: 900, Modified: 100, Origin: deviceB, Hash: "old"}
	newer := syncDevice{ID: deviceA, Revision: "1-1", Uploaded: 1, Modified: 300, Origin: deviceA, Hash: "new"}
	if got := newestSyncDevice([]syncDevice{older, newer}); got.Hash != "new" {
		t.Fatalf("上传时间不能让旧数据胜出：%+v", got)
	}
	if compareSyncVersions(local, deviceVersion(older)) <= 0 {
		t.Fatal("本机较新应该上传")
	}
	if compareSyncVersions(local, deviceVersion(newer)) >= 0 {
		t.Fatal("远端较新应该下载")
	}
}

func TestAutoSyncScheduleAndInvalidSettings(t *testing.T) {
	loc := time.FixedZone("UTC+8", 8*3600)
	now := time.Date(2026, 11, 17, 21, 10, 0, 0, loc)
	c := syncSettings{Schedule: "daily", DailyTime: "21:30", IntervalMinutes: 30, RetryLimit: 3, KeepSnapshots: 5}
	if got := nextSyncTime(c, now); !got.Equal(time.Date(2026, 11, 17, 21, 30, 0, 0, loc)) {
		t.Fatal(got)
	}
	if got := nextSyncTime(c, now.Add(time.Hour)); !got.Equal(time.Date(2026, 11, 18, 21, 30, 0, 0, loc)) {
		t.Fatal(got)
	}
	c.Schedule = "interval"
	if got := nextSyncTime(c, now); !got.Equal(now.Add(30 * time.Minute)) {
		t.Fatal(got)
	}
	c.IntervalMinutes = 0
	if validateSyncPlan(c) == nil {
		t.Fatal("不能接受零分钟间隔")
	}
	c.IntervalMinutes, c.DailyTime = 30, "25:00"
	if validateSyncPlan(c) == nil {
		t.Fatal("不能接受不存在的时间")
	}
}

func TestAutoSyncDoesNotRefreshVersionAfterRemoteRestoreOrRestart(t *testing.T) {
	state := syncCheckpoint{Target: "target", Version: syncVersion{Hash: "same", Modified: 100, Origin: deviceB}}
	// 恢复会改本机文件时间；内容相同就必须沿用远端原始时间。
	got := localSyncVersion(&state, "target", "same", 900, deviceA)
	if got.Modified != 100 || got.Origin != deviceB {
		t.Fatal(got)
	}
	state.Pending = &syncVersion{Hash: "downloaded", Modified: 200, Origin: deviceB}
	got = localSyncVersion(&state, "target", "downloaded", 1000, deviceA)
	if got.Modified != 200 || state.Pending != nil {
		t.Fatal("恢复中断后没有识别已落地的远端版本", got)
	}
	got = localSyncVersion(&state, "target", "edited", 1100, deviceA)
	if got.Modified != 1100 || got.Origin != deviceA {
		t.Fatal("本机新编辑没有创建新版本", got)
	}
}

func TestAutoSyncRejectsCorruptCloudAndChangedLocalBeforeReplacing(t *testing.T) {
	for _, failure := range []string{"metadata", "checksum", "local-edit"} {
		t.Run(failure, func(t *testing.T) {
			dir := t.TempDir()
			c, _ := loadSyncSettings(dir)
			c.Enabled = true
			c.DeviceID = deviceA
			if err := saveSyncSettings(dir, c); err != nil {
				t.Fatal(err)
			}
			b := &syncBackendFixture{data: []byte(`{"value":"keep"}`), modified: 100, fail: failure == "local-edit"}
			a, err := newAutoSync(dir, b)
			if err != nil {
				t.Fatal(err)
			}
			store := &memStore{files: map[string][]byte{}}
			remote := filepath.Join(t.TempDir(), "remote.json")
			os.WriteFile(remote, testRecordSnapshot([]byte(`{"value":"remote"}`), 200), 0600)
			hash, _ := snapshotHash(remote)
			device, err := uploadVersionedSnapshot(store, deviceB, "remote", remote, time.Now(), syncVersion{Modified: 200, Origin: deviceB, Hash: hash}, 5)
			if err != nil {
				t.Fatal(err)
			}
			if failure == "metadata" {
				store.files[device.snapshotKey()] = []byte(`{broken`)
			}
			if failure == "checksum" {
				store.files[device.snapshotKey()] = bytes.Replace(store.files[device.snapshotKey()], []byte(`"value":"remote"`), []byte(`"value":"broken"`), 1)
			}
			a.openStore = func(syncSettings) (remoteStore, error) { return store, nil }
			if _, err = a.cycle(c); err == nil {
				t.Fatal("不安全的同步应失败")
			}
			if b.imports != 0 || string(b.data) != `{"value":"keep"}` {
				t.Fatal("本机被覆盖")
			}
			if len(store.files) != 1 {
				t.Fatal("错误地用本机覆盖云端")
			}
		})
	}
}

func TestAutoSyncSharedGatePreventsOverlappingManualAndScheduledWork(t *testing.T) {
	dir := t.TempDir()
	c, _ := loadSyncSettings(dir)
	c.Enabled = true
	c.Automatic = true
	if err := saveSyncSettings(dir, c); err != nil {
		t.Fatal(err)
	}
	b := &syncBackendFixture{data: []byte(`{}`), modified: 100}
	a, err := newAutoSync(dir, b)
	if err != nil {
		t.Fatal(err)
	}
	a.openStore = func(syncSettings) (remoteStore, error) {
		t.Fatal("手动任务持锁时不能自动联网")
		return nil, nil
	}
	a.state.Next = time.Now().Add(-time.Minute)
	a.gate.Lock()
	a.run(time.Now(), false)
	a.run(time.Now(), true)
	a.gate.Unlock()
	if a.state.Count != 0 {
		t.Fatal("被互斥挡住的任务不能扣次数")
	}
}

func TestAutoSyncPageSavesPlanAndShowsRuntimeStatus(t *testing.T) {
	s, b := newTestSettings()
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.Enabled = true
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	manager, err := newAutoSync(s.syncDir, b)
	if err != nil {
		t.Fatal(err)
	}
	s.autoSync = manager
	s.autoSyncStatus = autoSyncState{Message: "已恢复 家里的电脑 的最新备份", Last: time.Date(2026, 11, 17, 10, 30, 0, 0, time.Local), Count: 2, Next: time.Date(2026, 11, 17, 11, 0, 0, 0, time.Local)}
	s.page = "sync"
	tt := ui.NewTester(s.view, 1000, 1800)
	tt.Frame()
	if err = tt.Click("自动同步"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	s.sync.Schedule, s.sync.DailyTime, s.sync.DailyLimit = "daily", "20:45", 12
	tt.Frame()
	if err = tt.Click("保存同步计划"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	saved, err := loadSyncSettings(s.syncDir)
	if err != nil || !saved.Automatic || saved.DailyTime != "20:45" || manager.config.DailyLimit != 12 {
		t.Fatalf("%+v %v", saved, err)
	}
	if !tt.HasText("已恢复 家里的电脑 的最新备份") || !tt.HasText("立即同步") {
		t.Fatal(tt.Texts())
	}
	// 同一个真实视图的最小窗口离屏图，检查新增控件在窄窗口的排布。
	if directory := os.Getenv("FREETYPE_SNAPSHOT_DIR"); directory != "" {
		for _, size := range []struct {
			name          string
			width, height int
		}{{"auto-sync-min", 780, 540}, {"auto-sync", 1000, 720}} {
			s.locateSetting(setting("sync/同步计划"))
			view := ui.NewTester(s.view, size.width, size.height)
			view.Frame()
			view.Frame()
			file, err := os.Create(filepath.Join(directory, size.name+".png"))
			if err != nil {
				t.Fatal(err)
			}
			if err = png.Encode(file, view.Image()); err != nil {
				file.Close()
				t.Fatal(err)
			}
			file.Close()
		}
	}
}

func TestAutoSyncTwoDevicesConvergeOverS3(t *testing.T) {
	remote := &fakeS3{t: t, objects: map[string][]byte{}}
	server := httptest.NewServer(remote)
	defer server.Close()
	makeDevice := func(id string, stamp int64, text string) (*autoSync, *syncBackendFixture) {
		t.Helper()
		dir := t.TempDir()
		c, _ := loadSyncSettings(dir)
		c.Enabled, c.Automatic, c.DeviceID = true, true, id
		c.Provider, c.S3Endpoint, c.S3Region, c.S3Bucket, c.S3Prefix = "s3", server.URL, "auto", "bucket", "FreeType"
		c.S3AccessKey, c.S3SecretKey = "key", "secret"
		if err := saveSyncSettings(dir, c); err != nil {
			t.Fatal(err)
		}
		b := &syncBackendFixture{data: []byte(text), modified: stamp}
		a, err := newAutoSync(dir, b)
		if err != nil {
			t.Fatal(err)
		}
		return a, b
	}
	a, ab := makeDevice(deviceA, 100, `{"value":"a"}`)
	b, bb := makeDevice(deviceB, 50, `{"value":"old"}`)
	for _, manager := range []*autoSync{a, b} {
		if _, err := manager.cycle(manager.config); err != nil {
			t.Fatal(err)
		}
	}
	if string(bb.data) != `{"value":"a"}` {
		t.Fatal("旧设备应下载新版本", string(bb.data))
	}
	bb.data = []byte(`{"value":"b"}`)
	bb.modified = 200
	for _, manager := range []*autoSync{b, a, a, b} {
		if _, err := manager.cycle(manager.config); err != nil {
			t.Fatal(err)
		}
	}
	if string(ab.data) != `{"value":"b"}` || !bytes.Equal(ab.data, bb.data) {
		t.Fatal("两台设备没有收敛")
	}
	if ab.imports != 1 || bb.imports != 1 {
		t.Fatal("收敛后仍重复恢复")
	}
}

func TestAutoSyncCanBeDisabledWhilePlanHasInvalidEdits(t *testing.T) {
	s, b := newTestSettings()
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.Enabled = true
	s.sync.Automatic = true
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	manager, err := newAutoSync(s.syncDir, b)
	if err != nil {
		t.Fatal(err)
	}
	s.autoSync = manager
	s.sync.DailyTime = "invalid"
	s.sync.Automatic = false
	if !s.saveSync() {
		t.Fatal("无效的未完成编辑不能阻止关闭自动同步", s.syncMessage)
	}
	if manager.config.Automatic || !manager.snapshot().Next.IsZero() {
		t.Fatal("仍在自动运行")
	}
	if _, err = newAutoSync(s.syncDir, b); err != nil {
		t.Fatal("关闭的无效计划不应阻止启动", err)
	}
}

func TestAutoSyncPublishesLaterRevertEvenWhenCloudHasSameContent(t *testing.T) {
	dir := t.TempDir()
	c, _ := loadSyncSettings(dir)
	c.Enabled = true
	c.DeviceID = deviceA
	if err := saveSyncSettings(dir, c); err != nil {
		t.Fatal(err)
	}
	b := &syncBackendFixture{data: []byte(`{"value":"original"}`), modified: 200}
	a, err := newAutoSync(dir, b)
	if err != nil {
		t.Fatal(err)
	}
	store := &memStore{files: map[string][]byte{}}
	file := filepath.Join(t.TempDir(), "remote.json")
	os.WriteFile(file, testRecordSnapshot(b.data, 100), 0600)
	hash, _ := snapshotHash(file)
	if _, err = uploadVersionedSnapshot(store, deviceB, "remote", file, time.Now(), syncVersion{Modified: 100, Origin: deviceB, Hash: hash}, 5); err != nil {
		t.Fatal(err)
	}
	a.openStore = func(syncSettings) (remoteStore, error) { return store, nil }
	// 本机先改为其他内容，再于 200 恢复原值。这个较晚的恢复也必须向其他设备传播。
	a.state.Checkpoint = syncCheckpoint{Target: syncTarget(c), Version: syncVersion{Modified: 150, Origin: deviceA, Hash: "intermediate"}}
	if _, err = a.cycle(c); err != nil {
		t.Fatal(err)
	}
	devices, err := readDevices(store, true)
	if err != nil {
		t.Fatal(err)
	}
	if newestSyncDevice(devices).Modified != 200 {
		t.Fatal("相同内容不能掩盖较晚的主动恢复", devices)
	}
}
