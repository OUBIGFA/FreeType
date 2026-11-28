package main

import (
	"image/png"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

func TestSettingsSearchLocatesMatchingRowAcrossPages(t *testing.T) {
	s, _ := newTestSettings()
	tt := ui.NewTester(s.view, 1000, 540)
	if err := tt.Click("搜索全部设置"); err != nil {
		t.Fatal(err)
	}
	tt.Type("自动加空格")
	if err := tt.Click("定位 拼音与按键 / 中英文之间自动加空格"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if s.page != "typing" {
		t.Fatalf("应跳转到拼音与按键：%s", s.page)
	}
	bounds, found := tt.Find("设置项：中英文之间自动加空格")
	if !found || bounds.Y < 32 || bounds.Y+bounds.H > 540 || bounds.X < 236 {
		t.Fatalf("应滚动到目标设置：%+v，存在=%v", bounds, found)
	}
}

func TestSettingsSearchRevealsHiddenOptionsWithoutChangingConfig(t *testing.T) {
	for _, title := range []string{"汉字和字母之间", "双拼方案", "双拼展示键位", "自动隐藏", "中英文状态", "左右方向键选联想词"} {
		s, b := newTestSettings()
		s.config.Autospace, s.config.Shuangpin, s.config.StatusBar = false, false, false
		s.config.DefaultMode = "global"
		tt := ui.NewTester(s.view, 1000, 540)
		if err := tt.Click("搜索全部设置"); err != nil {
			t.Fatal(err)
		}
		tt.Type(title)
		page := "拼音与按键"
		if title == "自动隐藏" || title == "中英文状态" {
			page = "常规"
		} else if title == "左右方向键选联想词" {
			page = "实用功能"
		}
		if err := tt.Click("定位 " + page + " / " + title); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
		bounds, found := tt.Find("设置项：" + title)
		if !found || bounds.Y < 32 || bounds.Y+bounds.H > 540 {
			t.Fatalf("应能定位隐藏选项 %s：%+v", title, bounds)
		}
		if s.config.Autospace || s.config.Shuangpin || s.config.StatusBar || s.config.PredictNext || s.config.DefaultMode != "global" || b.last("set_config") != nil {
			t.Fatal("搜索不能自行改变任何设置")
		}
	}
}

func TestSettingsSearchNoMatchesEscapeAndMinimumWindow(t *testing.T) {
	s, _ := newTestSettings()
	tt := ui.NewTester(s.view, 780, 540)
	tt.Frame()
	bounds, found := tt.Find("搜索全部设置")
	if !found || bounds.X < 0 || bounds.X+bounds.W > 236 || bounds.Y+bounds.H > 540 {
		t.Fatalf("侧边栏底部应显示搜索框：%+v", bounds)
	}
	if err := tt.Click("搜索全部设置"); err != nil {
		t.Fatal(err)
	}
	tt.Type("不存在的设置xyz")
	if !tt.HasText("没有找到相关设置") {
		t.Fatalf("应显示无结果提示：%q", tt.Texts())
	}
	tt.Key(0, ui.KeyEscape)
	if !tt.HasText("中英文切换") {
		t.Fatal("Esc 应退出搜索，返回当前页面")
	}
}

func TestEverySettingsSearchResultHasARealTarget(t *testing.T) {
	for _, item := range settingsItems {
		t.Run(item.ID, func(t *testing.T) {
			s, _ := newTestSettings()
			s.locateSetting(item)
			tt := ui.NewTester(s.view, 1000, 1400)
			tt.Frame()
			if _, found := tt.Find("设置项：" + item.Title); !found {
				t.Fatalf("搜索结果没有对应的实际设置：%s", item.ID)
			}
		})
	}
}

func TestSnapshots(t *testing.T) {
	dir := os.Getenv("FREETYPE_SNAPSHOT_DIR")
	if dir == "" {
		t.Skip("FREETYPE_SNAPSHOT_DIR 没设")
	}
	fontNames = func() []string { return []string{"Microsoft YaHei", "SimSun", "KaiTi"} }
	// 候选窗预览换成楷体、每页 9 个：看预览是不是真的跟着变
	{
		s, _ := newTestSettings()
		s.page, s.config.FontFamily, s.config.PageSize = "candidates", "KaiTi", 9
		tt := ui.NewTester(s.view, 1400, 720)
		tt.Frame()
		f, err := os.Create(filepath.Join(dir, "candidates-kaiti.png"))
		if err != nil {
			t.Fatal(err)
		}
		if err := png.Encode(f, tt.Image()); err != nil {
			t.Fatal(err)
		}
		f.Close()
	}
	for _, dark := range []bool{false, true} {
		for _, pg := range pages {
			s, _ := newTestSettings()
			if _, err := os.Stat("resources"); err == nil {
				s.resources = "resources"
			}
			s.page = pg.ID
			if pg.ID == "learned" {
				s.loadWords()
			}
			tt := ui.NewTester(s.view, 1400, 720)
			tt.SetDark(dark)
			tt.Frame()
			name := pg.ID
			if dark {
				name += "-dark"
			}
			f, err := os.Create(filepath.Join(dir, name+".png"))
			if err != nil {
				t.Fatal(err)
			}
			if err := png.Encode(f, tt.Image()); err != nil {
				t.Fatal(err)
			}
			f.Close()
		}
	}
}

func TestNewSettingsLayoutsSnapshots(t *testing.T) {
	directory := os.Getenv("FREETYPE_SNAPSHOT_DIR")
	if directory == "" {
		t.Skip("FREETYPE_SNAPSHOT_DIR 没设")
	}
	for _, sample := range []struct {
		name, target, query string
		width, height       int
	}{
		{"backup-min-window", "sync/全部设置和数据", "", 780, 540},
		{"common-min-window", "common/导入或导出", "", 780, 540},
		{"search-hidden-option", "typing/汉字和字母之间", "", 1000, 540},
		{"mixed-original-style", "typing/中英文之间自动加空格", "", 1000, 600},
		{"about-min-window", "about/自动更新", "", 780, 540},
		{"after-update-min-window", "about/更新安装后显示设置页", "", 780, 540},
		{"about-updates-ready", "about/检查更新", "", 1000, 540},
		{"search-results", "", "中英 空格", 1000, 720},
		{"typing-fuzzy-groups-min-window", "typing/模糊音分组", "", 780, 540},
		{"typing-arrow-keys", "typing/左右方向键", "", 1000, 540},
		{"features-predict-and-presets", "features/上屏联想", "", 1000, 1100},
		{"sync-webdav", "sync/同步服务", "", 1000, 1100},
		{"sync-s3-min-window", "sync/上传与恢复", "", 780, 900},
		{"sync-connection-min", "sync/同步服务", "", 780, 720},
		{"sync-status-min", "sync/同步计划", "", 780, 720},
		{"sync-status", "sync/同步计划", "", 1000, 720},
	} {
		s, _ := newTestSettings()
		s.config.Autospace = false
		if sample.name == "mixed-original-style" {
			s.config.Autospace = true
		}
		s.common = []string{"您好，欢迎使用 FreeType。", "第一行\n第二行"}
		if sample.name == "about-updates-ready" {
			s.updateStatus = updateState{Version: "0.2.0", Message: "更新已下载，可立即更新；开启自动更新后，下次启动时安装", Ready: true}
		}
		s.onCheckUpdates, s.onUpdateNow = func() {}, func() {}
		s.syncDir = t.TempDir()
		switch sample.name {
		case "features-predict-and-presets":
			s.config.PredictNext, s.config.AppPresets = true, true
			s.config.AppPresetRules = []AppPreset{{App: "code.exe", Mode: "english"}}
		case "sync-webdav", "sync-s3-min-window", "sync-connection-min", "sync-status-min", "sync-status":
			s.loadSync()
			s.sync.Enabled, s.sync.WebDAVURL, s.sync.WebDAVUser = true, "https://dav.jianguoyun.com/dav/FreeType", "me@example.com"
			s.sync.DeviceID = "0123456789abcdef0123456789abcdef"
			s.syncDevices = []syncDevice{
				{ID: s.sync.DeviceID, Hostname: "OFFICE-PC", Uploaded: 1794800000, Size: 1536000},
				{ID: "fedcba9876543210fedcba9876543210", Hostname: "家里的电脑", Uploaded: 1794700000, Size: 820000},
			}
			s.syncMessage = "已上传；1.5 MB"
			if sample.name == "sync-connection-min" {
				s.syncMessage, s.syncMessageArea = "连接成功，可以上传和恢复", "connection"
			}
			s.sync.Automatic = true
			s.autoSyncStatus = autoSyncState{Message: "已上传本机最新备份", Last: time.Date(2026, 11, 17, 14, 30, 5, 0, time.Local), Next: time.Date(2026, 11, 17, 15, 0, 5, 0, time.Local), Count: 3}
			if sample.name == "sync-s3-min-window" {
				s.sync.Provider = "s3"
			}
		}
		if _, err := os.Stat("resources"); err == nil {
			s.resources = "resources"
		}
		if sample.target != "" {
			s.locateSetting(setting(sample.target))
		}
		s.settingQuery = sample.query
		tt := ui.NewTester(s.view, sample.width, sample.height)
		tt.Frame()
		file, err := os.Create(filepath.Join(directory, sample.name+".png"))
		if err != nil {
			t.Fatal(err)
		}
		if err := png.Encode(file, tt.Image()); err != nil {
			file.Close()
			t.Fatal(err)
		}
		file.Close()
	}
}
