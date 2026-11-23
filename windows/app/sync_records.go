package main

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

func preserveSyncBackup(dir, local string) error {
	data, err := os.ReadFile(local)
	if err != nil {
		return err
	}
	dir = filepath.Join(dir, "sync-backups")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, fmt.Sprintf("before-sync-%d.json", time.Now().UnixNano())), data, 0600)
}

// 新服务提供记录级同步。每台设备的最新快照都可能有独立修改，不能只取全局最新一份。
func (a *autoSync) cycleRecords(c syncSettings, store remoteStore, devices []syncDevice, local, work string, version syncVersion) (string, error) {
	added, updated, deleted := 0, 0, 0
	backedUp := false
	var ownHash string
	for i, device := range devices {
		if device.ID == c.DeviceID {
			ownHash = device.Hash
		}
		if device.Hash == version.Hash {
			continue
		}
		if !a.stillConfigured(c) {
			return "", errors.New("同步设置已改变，本次同步已停止")
		}
		incoming := filepath.Join(work, fmt.Sprintf("incoming-%d.json", i))
		if err := downloadExactSnapshot(store, device, incoming); err != nil {
			return "", err
		}
		preview, err := a.backend.Call(op("preview_sync_merge", "path", incoming))
		if err != nil {
			return "", err
		}
		if preview.SyncMerge == nil {
			return "", errors.New("服务未返回同步差异，请更新服务")
		}
		p := preview.SyncMerge
		if p.Versions == 0 {
			continue
		}
		if !backedUp {
			if err := preserveSyncBackup(a.dir, local); err != nil {
				return "", err
			}
			backedUp = true
		}
		// 下载和预览期间用户仍可关闭同步或更换目标；应用文件前再核对一次。
		if !a.stillConfigured(c) {
			return "", errors.New("同步设置已改变，本次同步已停止")
		}
		response, err := a.backend.Call(op("merge_sync_settings", "path", incoming, "expected_path", local))
		if err != nil {
			return "", err
		}
		if response.SyncMerge == nil {
			return "", errors.New("服务未返回合并结果")
		}
		added += response.SyncMerge.Added
		updated += response.SyncMerge.Updated
		deleted += response.SyncMerge.Deleted
		if a.restored != nil {
			a.restored(response)
		}
		fresh, err := a.backend.Call(op("export_sync_settings", "path", local))
		if err != nil {
			return "", err
		}
		hash, err := snapshotHash(local)
		if err != nil {
			return "", err
		}
		version = syncVersion{Modified: fresh.ModifiedUnixMS, Origin: c.DeviceID, Hash: hash}
	}
	if ownHash != version.Hash {
		if !a.stillConfigured(c) {
			return "", errors.New("同步设置已改变，本次同步已停止")
		}
		hostname, _ := os.Hostname()
		if _, err := uploadVersionedSnapshot(store, c.DeviceID, hostname, local, time.Now(), version, c.KeepSnapshots); err != nil {
			return "", err
		}
	}
	a.mu.Lock()
	a.state.Checkpoint.Version, a.state.Checkpoint.Pending = version, nil
	err := a.saveLocked()
	a.mu.Unlock()
	if err != nil {
		return "", err
	}
	if added+updated+deleted == 0 {
		return "同步完成；没有需要合并的修改", nil
	}
	return fmt.Sprintf("同步完成；新增 %d 项，更新 %d 项，删除 %d 项", added, updated, deleted), nil
}
