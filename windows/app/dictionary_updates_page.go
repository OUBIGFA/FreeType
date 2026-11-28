package main

import (
	"context"
	"time"

	"github.com/egoist/mygo/ui"
)

// dataVersionLabel 显示词库版本；空串是随安装包的词库。
func dataVersionLabel(version, bundled string) string {
	switch version {
	case "":
		return "随安装包 " + bundled
	case "bundled":
		return "随安装包的词库"
	}
	return version
}

// dictionaryUpdateSection 显示更新包状态和操作；网络与校验在后台进行，不阻塞界面和输入。
func (s *settings) dictionaryUpdateSection(c *ui.Context) {
	manager, state := s.dataUpdates, s.dataUpdate
	status := state.Status
	section(c, "词库更新", func() {
		current := "正在读取"
		if state.Loaded {
			current = dataVersionLabel(status.Active, status.Bundled)
		}
		s.settingRowValue(c, "dicts/当前词库", current, nil)
		pending := "无"
		if status.Pending != "" {
			pending = dataVersionLabel(status.Pending, status.Bundled) + "；下次服务启动时生效"
		}
		s.settingRowValue(c, "dicts/待生效词库", pending, func() {
			if ui.Button(c, "取消").Label("取消待生效的词库").Disabled(manager == nil || state.Busy || status.Pending == "").Clicked() {
				go func() { _ = manager.cancelPending() }()
			}
		})
		s.settingRowValue(c, "dicts/检查词库更新", state.Message, func() {
			if ui.Button(c, "检查词库更新").Disabled(manager == nil || state.Busy).Clicked() {
				go func() {
					ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
					defer cancel()
					_ = manager.check(ctx)
				}()
			}
			if ui.Button(c, "下载更新").Label("下载词库更新").Disabled(manager == nil || state.Busy || state.Available == "").Clicked() {
				go func() {
					ctx, cancel := context.WithTimeout(context.Background(), 30*time.Minute)
					defer cancel()
					_ = manager.downloadAvailable(ctx)
				}()
			}
			if ui.Button(c, "恢复上一版").Label("恢复上一版词库").Disabled(manager == nil || state.Busy || status.Current == "").Clicked() {
				go func() { _ = manager.restorePrevious() }()
			}
		})
		if s.loaded {
			if s.settingSwitch(c, "dicts/自动更新词库", false, &s.config.DictionaryAutoUpdate) {
				s.saveConfig()
			}
		}
	})
	if status.FailedVersion != "" {
		hint(c, "词库 "+status.FailedVersion+" 未能生效："+status.FailedReason+"；已继续使用其他有效词库")
	}
	if s.config.Lexicon != "" {
		hint(c, "已选择自定义主词库；更新包仍提供英文词表，主词库继续使用所选文件")
	}
}
