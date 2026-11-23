package main

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

type pauseAfterPreview struct {
	*syncBackendFixture
	pause func()
}

func (b *pauseAfterPreview) Call(request Request) (Response, error) {
	response, err := b.syncBackendFixture.Call(request)
	if request["op"] == "preview_sync_merge" && b.pause != nil {
		b.pause()
	}
	return response, err
}

func TestSyncDisabledAfterPreviewCannotApplyDownloadedData(t *testing.T) {
	dir := t.TempDir()
	c, err := loadSyncSettings(dir)
	if err != nil {
		t.Fatal(err)
	}
	c.Enabled, c.DeviceID = true, deviceA
	if err := saveSyncSettings(dir, c); err != nil {
		t.Fatal(err)
	}
	backend := &pauseAfterPreview{syncBackendFixture: &syncBackendFixture{data: []byte(`{"value":"keep"}`), modified: 100}}
	manager, err := newAutoSync(dir, backend)
	if err != nil {
		t.Fatal(err)
	}
	store := &memStore{files: map[string][]byte{}}
	incoming := filepath.Join(t.TempDir(), "incoming.json")
	if err := os.WriteFile(incoming, testRecordSnapshot([]byte(`{"value":"remote"}`), 200), 0600); err != nil {
		t.Fatal(err)
	}
	hash, _ := snapshotHash(incoming)
	if _, err := uploadVersionedSnapshot(store, deviceB, "remote", incoming, time.Now(), syncVersion{Modified: 200, Origin: deviceB, Hash: hash}, 5); err != nil {
		t.Fatal(err)
	}
	manager.openStore = func(syncSettings) (remoteStore, error) { return store, nil }
	backend.pause = func() { changed := c; changed.Enabled = false; manager.configure(changed) }
	if _, err := manager.cycle(c); err == nil {
		t.Fatal("关闭同步后应停止恢复")
	}
	if backend.imports != 0 || string(backend.data) != `{"value":"keep"}` {
		t.Fatalf("关闭同步后仍恢复了下载的内容：%s", backend.data)
	}
}
