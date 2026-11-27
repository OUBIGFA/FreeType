package main

import "fmt"

// resetSettings 只重置设置，沿用服务端缺省字段的默认值，不删除用户数据。
func (s *settings) resetSettings() {
	if !s.loaded || s.resetting || s.syncBusy != "" {
		return
	}
	manager := s.autoSync
	// 与同步共用锁，防止旧快照在恢复后重新覆盖默认值。
	if manager != nil && !manager.gate.TryLock() {
		s.notice = "同步正在进行，请稍后再恢复出厂设置"
		return
	}
	s.loadSync()
	dir := s.syncDataDir()
	restoredSync := defaultSyncSettings()
	restoredSync.DeviceID = s.sync.DeviceID
	if s.pendingSaveTimer != nil {
		s.pendingSaveTimer.Stop()
		s.pendingSaveTimer = nil
	}
	s.pendingSave = false
	s.resetting = true
	s.configRevision++
	previous := s.configSaveTail
	done := make(chan struct{})
	s.configSaveTail = done
	s.background(func() func() {
		defer close(done)
		if manager != nil {
			defer manager.gate.Unlock()
		}
		// 排在已提交的设置修改之后；重置期间界面暂停编辑。
		if previous != nil {
			<-previous
		}
		response, err := s.backend.Call(op("set_config", "config", map[string]any{}))
		if err == nil && response.Config == nil {
			err = fmt.Errorf("服务未返回恢复后的设置")
		}
		var syncErr error
		if err == nil {
			syncErr = saveSyncSettings(dir, restoredSync)
			if syncErr == nil && manager != nil {
				manager.configure(restoredSync)
			}
		}
		return func() {
			s.resetting = false
			if err != nil {
				s.notice = "恢复出厂设置失败：" + err.Error()
				return
			}
			s.config = *response.Config
			s.shuangpinExample = response.ShuangpinExample
			s.notice = "已恢复出厂设置"
			if syncErr != nil {
				// 两处保存可能分别失败，明确报告实际完成的范围，允许重试。
				s.notice = "输入法设置已恢复；同步设置恢复失败：" + syncErr.Error()
			} else {
				s.sync, s.syncLoaded = restoredSync, true
				s.syncDevices, s.syncListed = nil, false
				s.syncMessage, s.syncMessageArea, s.syncFailed = "", "", false
				s.syncFailedDevice = nil
				s.confirmSyncResetCloud, s.confirmForceRestore = false, false
				s.cancelSyncPreview()
			}
			s.updateBrandIcon()
			if s.onTray != nil {
				s.onTray(s.config.TrayIcon)
			}
			if s.onAutomaticUpdate != nil {
				s.onAutomaticUpdate(s.config.AutoUpdate)
			}
		}
	})
}
