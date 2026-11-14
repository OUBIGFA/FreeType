package main

import (
	"encoding/json"
	"testing"
)

func TestAboutUpdateControlsSaveDefaultOffAndUseRealActions(t *testing.T) {
	s, backend := newTestSettings()
	checks, installs := 0, 0
	var automatic []bool
	s.onCheckUpdates = func() { checks++ }
	s.onUpdateNow = func() { installs++ }
	s.onAutomaticUpdate = func(on bool) { automatic = append(automatic, on) }
	tt := open(t, s, "关于")
	if s.config.AutoUpdate {
		t.Fatal("自动更新必须默认关闭")
	}
	if err := tt.Click("检查软件更新"); err != nil {
		t.Fatal(err)
	}
	if checks != 1 {
		t.Fatal("检查按钮必须执行实际检查操作")
	}
	_ = tt.Click("立即更新")
	if installs != 0 {
		t.Fatal("没有可用更新时不能触发安装")
	}
	if err := tt.Click("自动更新"); err != nil {
		t.Fatal(err)
	}
	data, _ := json.Marshal(backend.last("set_config")["config"])
	var saved Config
	if err := json.Unmarshal(data, &saved); err != nil {
		t.Fatal(err)
	}
	if !saved.AutoUpdate || len(automatic) != 1 || !automatic[0] {
		t.Fatal("开关必须保存并启动后台更新")
	}
	s.updateStatus = updateState{Version: "0.2.0", Message: "可更新到 0.2.0"}
	tt.Frame()
	if err := tt.Click("立即更新"); err != nil {
		t.Fatal(err)
	}
	if installs != 1 {
		t.Fatal("立即更新按钮必须执行下载及安装操作")
	}
	s.updateStatus.Busy = true
	tt.Frame()
	_ = tt.Click("检查软件更新")
	_ = tt.Click("立即更新")
	if checks != 1 || installs != 1 {
		t.Fatal("检查或下载期间必须防止重复操作")
	}
}

func TestShowSettingsAfterUpdateSwitchPersistsWithoutEnablingAutomaticUpdates(t *testing.T) {
	s, backend := newTestSettings()
	tt := open(t, s, "关于")
	if s.config.ShowSettingsAfterUpdate {
		t.Fatal("安装后显示设置页必须默认关闭")
	}
	for _, want := range []bool{true, false} {
		if err := tt.Click("更新安装后显示设置页"); err != nil {
			t.Fatal(err)
		}
		data, err := json.Marshal(backend.last("set_config")["config"])
		if err != nil {
			t.Fatal(err)
		}
		var saved map[string]any
		if err := json.Unmarshal(data, &saved); err != nil {
			t.Fatal(err)
		}
		if saved["show_settings_after_update"] != want || saved["auto_update"] != false {
			t.Fatalf("显示设置页的开关应单独保存：%s", data)
		}
		s.refresh()
		tt.Frame()
	}
}
