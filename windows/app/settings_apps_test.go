package main

import (
	"testing"
)

func TestAboutPageOpensFolders(t *testing.T) {
	s, _ := newTestSettings()
	var opened []string
	s.openFolder = func(path string) { opened = append(opened, path) }
	tt := open(t, s, "关于")
	if err := tt.Click("打开文件夹"); err != nil {
		t.Fatal(err)
	}
	if len(opened) != 1 || opened[0] != `C:\data` {
		t.Fatalf("打开的文件夹不对：%v", opened)
	}
}

func TestTraySwitchDefaultsOnAndTellsTheTrayApp(t *testing.T) {
	s, b := newTestSettings()
	var shown []bool
	s.onTray = func(show bool) { shown = append(shown, show) }
	tt := open(t, s, "")
	if !b.config.TrayIcon {
		t.Fatal("托盘图标默认应开着")
	}
	if err := tt.Click("显示托盘图标"); err != nil {
		t.Fatal(err)
	}
	if b.config.TrayIcon || len(shown) != 1 || shown[0] {
		t.Fatalf("关掉托盘图标没生效：%+v %v", b.config.TrayIcon, shown)
	}
}

func TestAppPresetRulesAreSavedUpdatedAndPersisted(t *testing.T) {
	s, b := newTestSettings()
	s.config.AppPresets = true
	s.newPresetApp = `C:\\Program Files\\Code.EXE`
	s.newPresetMode = "english"
	s.addPreset()
	if len(b.config.AppPresetRules) != 1 || b.config.AppPresetRules[0] != (AppPreset{App: "code.exe", Mode: "english"}) {
		t.Fatalf("指定程序规则没有保存：%+v", b.config.AppPresetRules)
	}
	// 同一程序再次添加应更新，而不是重复添加；这就是重启后仍保留且不用反复添加的配置。
	s.newPresetApp = "code.exe"
	s.newPresetMode = "chinese"
	s.addPreset()
	if len(b.config.AppPresetRules) != 1 || b.config.AppPresetRules[0].Mode != "chinese" {
		t.Fatalf("重复程序没有更新原规则：%+v", b.config.AppPresetRules)
	}
	if len(s.apps) != 2 || s.apps[0].App != "code.exe" {
		t.Fatalf("没有读取当前打开的程序：%+v", s.apps)
	}
}

func TestAppPresetSettingsPageShowsWindowPickerAndRules(t *testing.T) {
	s, b := newTestSettings()
	s.config.AppPresets = true
	b.config.AppPresets = true
	tt := open(t, s, "实用功能")
	tt.Frame()
	for _, text := range []string{"指定软件规则", "刷新打开的窗口", "手动输入"} {
		if !tt.HasText(text) {
			t.Fatalf("打开指定软件设置后没有「%s」：%q", text, tt.Texts())
		}
	}
	if !b.config.AppPresets {
		t.Fatal("指定软件总开关没有保存")
	}
}

func TestVimApplicationRulePersistsWithoutEnablingApplicationPresets(t *testing.T) {
	s, b := newTestSettings()
	s.newVimApp = `C:\Editors\Code.EXE`
	tt := open(t, s, "实用功能")
	tt.Frame()
	if err := tt.Click("添加 Vim 程序"); err != nil {
		t.Fatal(err)
	}
	if len(b.config.VimModeApps) != 1 || b.config.VimModeApps[0] != "code.exe" || b.config.AppPresets {
		t.Fatalf("Vim 规则应独立保存：%+v", b.config)
	}
}

func TestAppPresetRulesCanBeEditedWithoutClipboardHistory(t *testing.T) {
	s, b := newTestSettings()
	s.config.AppPresets, b.config.AppPresets = true, true
	if s.config.ClipboardHistory {
		t.Fatal("这个测试要在剪贴板历史关闭时进行")
	}
	for _, id := range []string{"features/指定软件规则", "features/指定软件规则列表"} {
		if reason := s.unavailableReason(id); reason != "" {
			t.Fatalf("%s 被禁用：%s", id, reason)
		}
	}
	s.newPresetApp = "code.exe"
	tt := open(t, s, "实用功能")
	tt.Frame()
	if err := tt.Click("添加 / 更新"); err != nil {
		t.Fatal(err)
	}
	if len(b.config.AppPresetRules) != 1 || b.config.AppPresetRules[0].App != "code.exe" {
		t.Fatalf("规则区点不动：%+v", b.config.AppPresetRules)
	}
	// 原来的前提仍然有效
	s.config.AppPresets = false
	if s.unavailableReason("features/指定软件规则") == "" || s.unavailableReason("features/清空剪贴板历史") == "" {
		t.Fatal("总开关关着时规则区、没开剪贴板历史时清空按钮应不可用")
	}
}

func TestPredictNextIsOffByDefaultAndSavedFromFeaturesPage(t *testing.T) {
	s, b := newTestSettings()
	if s.config.PredictNext || s.config.PredictDigits || !s.config.PredictSpace || !s.config.PredictDoubleSpace || !s.config.PredictArrows {
		t.Fatalf("上屏联想和数字选词默认关，空格、双击空格和左右选词默认开：%+v", s.config)
	}
	tt := open(t, s, "实用功能")
	labels := []string{"数字键选联想词", "空格选联想词", "双击空格键选词", "左右方向键选联想词"}
	for _, label := range labels {
		if tt.HasText(label) {
			t.Fatalf("没开上屏联想时不应显示 %q", label)
		}
	}
	if err := tt.Click("上屏联想"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	for _, label := range labels {
		if !tt.HasText(label) {
			t.Fatalf("开启上屏联想后应显示 %q", label)
		}
	}
	if tt.HasText("自定义联想选词键") || len(findSettings("自定义联想选词键")) != 0 {
		t.Error("已移除的自定义选词键不应出现在设置页或搜索结果中")
	}
	if !b.config.PredictNext || b.config.PredictDigits || !b.config.PredictSpace || !b.config.PredictDoubleSpace || !b.config.PredictArrows {
		t.Fatalf("开启上屏联想应保存选词键的默认值：%+v", b.config)
	}
	for _, label := range []string{"数字键选联想词", "双击空格键选词", "空格选联想词", "左右方向键选联想词"} {
		if err := tt.Click(label); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
	}
	if tt.HasText("双击空格键选词") {
		t.Fatal("关闭空格选词时应隐藏双击子项")
	}
	if !b.config.PredictDigits || b.config.PredictSpace || b.config.PredictDoubleSpace || b.config.PredictArrows {
		t.Fatalf("子选项没有分别保存：%+v", b.config)
	}
	// 重新打开总开关时保留已保存的按键偏好。
	for range 2 {
		if err := tt.Click("上屏联想"); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
	}
	if !b.config.PredictNext || !b.config.PredictDigits || b.config.PredictSpace || b.config.PredictDoubleSpace || b.config.PredictArrows {
		t.Fatalf("重新开启联想不能覆盖已保存的选词键偏好：%+v", b.config)
	}
}

func TestSyncProblemClassificationAndRecoveryActions(t *testing.T) {
	cases := []struct {
		msg  string
		want syncProblemKind
	}{
		{"同步记录与备份正文不一致", syncProblemRecordsMismatch},
		{"备份密码不正确，或者云端备份已损坏", syncProblemPassword},
		{"云端备份已加密；请在云端同步中填写该备份的密码", syncProblemPassword},
		{"云端备份内容损坏", syncProblemCorruptRemote},
		{"云端备份已变化，请重新同步", syncProblemTransient},
		{"本机数据在下载期间已修改，将重新比较版本", syncProblemTransient},
		{"上传失败：HTTP 500", syncProblemConnection},
		{"未知失败", syncProblemOther},
	}
	for _, item := range cases {
		if got := classifySyncProblem(item.msg); got != item.want {
			t.Errorf("classifySyncProblem(%q) = %v, want %v", item.msg, got, item.want)
		}
	}
}
