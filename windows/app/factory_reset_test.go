package main

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

func TestFactoryResetRequiresConfirmationAndPreservesUserData(t *testing.T) {
	s, backend := newTestSettings()
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.Enabled, s.sync.Automatic = true, true
	s.sync.WebDAVURL, s.sync.WebDAVUser, s.sync.WebDAVPassword = "https://example.invalid/dav", "account", "password"
	s.sync.Encrypt, s.sync.EncryptPassword = true, "backup-password"
	s.sync.IntervalMinutes = 120
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	s.config.FontSize, s.config.TrayIcon, s.config.AutoUpdate = 26, false, true
	s.config.Shuangpin, s.config.Lexicon = true, "custom.tsv"
	s.config.AppPresetRules = []AppPreset{{App: "editor.exe", Mode: "english"}}
	s.saveConfig()
	before, syncBefore := s.config, s.sync
	words, phrases := append([]UserWord{}, backend.words...), append([]QuickPhrase{}, backend.phrases...)
	backend.calls = nil
	view := open(t, s, "数据与同步")
	if err := view.Click("恢复出厂设置"); err != nil {
		t.Fatal(err)
	}
	if backend.last("set_config") != nil || !reflect.DeepEqual(s.config, before) || s.sync != syncBefore {
		t.Fatal("打开确认框不能修改设置")
	}
	if err := view.Click("取消"); err != nil {
		t.Fatal(err)
	}
	if backend.last("set_config") != nil || !reflect.DeepEqual(s.config, before) || s.sync != syncBefore {
		t.Fatal("取消后必须保留全部设置")
	}
	if err := view.Click("恢复出厂设置"); err != nil {
		t.Fatal(err)
	}
	var tray, automatic bool
	var brand string
	s.onTray = func(on bool) { tray = on }
	s.onAutomaticUpdate = func(on bool) { automatic = on }
	s.onBrandIcon = func(c Config) error { brand = c.BrandIcon; return nil }
	if err := view.Click("确认恢复"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(s.config, defaultConfig()) || !reflect.DeepEqual(backend.config, defaultConfig()) {
		t.Fatalf("确认后应恢复全部默认设置：%+v", s.config)
	}
	if !tray || automatic || brand != "default" {
		t.Fatalf("托盘、图标与自动更新没有生效：%v %v %q", tray, automatic, brand)
	}
	wantSync, err := loadSyncSettings(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	wantSync.DeviceID = syncBefore.DeviceID
	savedSync, err := loadSyncSettings(s.syncDir)
	if err != nil || savedSync != wantSync || s.sync != wantSync {
		t.Fatalf("同步设置和凭据应恢复默认并保留设备身份：%+v %v", savedSync, err)
	}
	if !reflect.DeepEqual(backend.words, words) || !reflect.DeepEqual(backend.phrases, phrases) {
		t.Fatal("恢复设置不能清除学习词和快捷短语")
	}
	for _, request := range backend.calls {
		if request["op"] != "set_config" {
			t.Fatalf("恢复设置不应修改其他用户数据：%v", request["op"])
		}
	}
}

func TestFactoryResetStopsAutomaticSyncAndWaitsForActiveSync(t *testing.T) {
	s, backend := newTestSettings()
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.Enabled, s.sync.Automatic = true, true
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	manager, err := newAutoSync(s.syncDir, backend)
	if err != nil {
		t.Fatal(err)
	}
	s.autoSync = manager
	manager.gate.Lock()
	s.resetSettings()
	manager.gate.Unlock()
	if backend.last("set_config") != nil || !s.sync.Enabled {
		t.Fatal("正在同步时不能恢复出厂设置")
	}
	s.resetSettings()
	if manager.config.Enabled || manager.config.Automatic || !manager.snapshot().Next.IsZero() {
		t.Fatal("恢复后必须停止自动同步和后续计划")
	}
}

// 模拟一次尚未返回的真实管道保存，只延迟应答，不改变保存内容。
type resetOrderingBackend struct {
	delayedConfigBackend
}

func (b *resetOrderingBackend) Call(request Request) (Response, error) {
	if _, empty := request["config"].(map[string]any); empty {
		request = op("set_config", "config", defaultConfig())
	}
	return b.delayedConfigBackend.Call(request)
}

func TestFactoryResetFollowsPendingSaveAndCancelsDebouncedEdits(t *testing.T) {
	backend := &resetOrderingBackend{delayedConfigBackend{
		requests: make(chan Config, 2), release: make(chan struct{}, 2),
	}}
	s := newSettings(backend, "0.0.1")
	s.loaded, s.syncDir = true, t.TempDir()
	changes := make(chan func(), 4)
	s.apply = func(change func()) { changes <- change }
	s.config.FontSize = 26
	s.saveConfig()
	<-backend.requests
	s.config.FontSize = 30
	s.scheduleSaveConfig()
	s.resetSettings()
	if s.pendingSave || s.pendingSaveTimer != nil {
		t.Fatal("恢复时必须取消尚未发送的延迟保存")
	}
	select {
	case <-backend.requests:
		t.Fatal("恢复请求不能越过之前的保存")
	case <-time.After(30 * time.Millisecond):
	}
	backend.release <- struct{}{}
	select {
	case restored := <-backend.requests:
		if restored.FontSize != 18 {
			t.Fatal("最后一条请求必须保存默认值")
		}
	case <-time.After(time.Second):
		t.Fatal("恢复请求没有继续执行")
	}
	backend.release <- struct{}{}
	for range 2 {
		select {
		case change := <-changes:
			change()
		case <-time.After(time.Second):
			t.Fatal("设置应答没有返回")
		}
	}
	if s.resetting || s.config.FontSize != 18 {
		t.Fatal("旧保存应答覆盖了恢复后的默认值")
	}
}

func TestFactoryResetDialogFitsSettingsWindow(t *testing.T) {
	for _, dark := range []bool{false, true} {
		s, _ := newTestSettings()
		s.syncDir, s.page = t.TempDir(), "sync"
		view := ui.NewTester(s.view, 900, 700)
		view.SetDark(dark)
		view.Frame()
		if !dark {
			saveUIFrame(t, "factory-reset-page", view)
		}
		if err := view.Click("恢复出厂设置"); err != nil {
			t.Fatal(err)
		}
		for _, label := range []string{"取消", "确认恢复"} {
			box, found := view.Find(label)
			if !found || box.X < 0 || box.Y < 0 || box.X+box.W > 900 || box.Y+box.H > 700 {
				t.Fatalf("确认框按钮应位于窗口内：%s %+v", label, box)
			}
		}
		if !dark {
			saveUIFrame(t, "factory-reset-confirm", view)
		}
	}
}

func TestFactoryResetReportsFailuresWithoutClaimingSuccess(t *testing.T) {
	for _, fail := range []string{"service", "sync"} {
		t.Run(fail, func(t *testing.T) {
			s, backend := newTestSettings()
			s.syncDir = t.TempDir()
			s.loadSync()
			s.sync.WebDAVUser = "keep-account"
			if !s.saveSync() {
				t.Fatal(s.syncMessage)
			}
			before := s.config
			if fail == "service" {
				backend.failRequest = "set_config"
			} else {
				// A directory at the atomic-write path makes the real disk write fail.
				if err := os.Mkdir(filepath.Join(s.syncDir, "sync.json.tmp"), 0700); err != nil {
					t.Fatal(err)
				}
			}
			view := open(t, s, "数据与同步")
			if err := view.Click("恢复出厂设置"); err != nil {
				t.Fatal(err)
			}
			if err := view.Click("确认恢复"); err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(strings.Join(view.Texts(), " "), "失败") {
				t.Fatal("保存失败必须向用户显示错误")
			}
			if fail == "service" && !reflect.DeepEqual(s.config, before) {
				t.Fatal("服务保存失败不能把界面伪装成默认值")
			}
			saved, err := loadSyncSettings(s.syncDir)
			if err != nil || saved.WebDAVUser != "keep-account" || s.sync.WebDAVUser != "keep-account" {
				t.Fatalf("失败时应保留未重置的同步设置：%+v %v", saved, err)
			}
		})
	}
}
