package main

import (
	"encoding/json"
	"github.com/egoist/mygo/ui"
	"image/png"
	"os"
	"path/filepath"
	"testing"
)

func TestCandidateDiagnosticsViewSnapshots(t *testing.T) {
	path := os.Getenv("FREETYPE_DIAGNOSTICS_SNAPSHOTS")
	if path == "" {
		t.Skip("snapshot output not requested")
	}
	if err := os.MkdirAll(path, 0700); err != nil {
		t.Fatal(err)
	}
	s, _ := newTestSettings()
	view := ui.NewTester(s.candidateDiagnosticsSection, 708, 360)
	for _, name := range []string{"section", "confirmation"} {
		if name == "confirmation" {
			view.Click("记录候选诊断")
			view.Frame()
		}
		file, err := os.Create(filepath.Join(path, name+".png"))
		if err != nil {
			t.Fatal(err)
		}
		err = png.Encode(file, view.Image())
		file.Close()
		if err != nil {
			t.Fatal(err)
		}
	}
}

func diagnosticEnabled(config Config) bool {
	data, _ := json.Marshal(config)
	var fields map[string]any
	_ = json.Unmarshal(data, &fields)
	return fields["candidate_diagnostics"] == true
}

func TestCandidateDiagnosticsRequiresConfirmationAndCanStop(t *testing.T) {
	s, backend := newTestSettings()
	view := ui.NewTester(s.aboutPage, 900, 1800)
	if _, found := view.Find("记录候选诊断"); !found {
		t.Fatal("candidate diagnostics opt-in is missing")
	}
	view.Click("记录候选诊断")
	if diagnosticEnabled(backend.config) {
		t.Fatal("opening confirmation must not enable recording")
	}
	if !view.HasText("会记录正文、拼音、上下文、候选和应用名称；仅保存在本机，普通诊断导出不包含这些内容") {
		t.Fatal("confirmation must explain recorded content")
	}
	view.Click("取消")
	if diagnosticEnabled(backend.config) {
		t.Fatal("cancel must keep recording disabled")
	}
	view.Click("记录候选诊断")
	view.Click("确认开启")
	if !diagnosticEnabled(backend.config) {
		t.Fatal("confirmed opt-in was not saved")
	}
	backend.failRequest = "set_config"
	view.Click("记录候选诊断")
	if !diagnosticEnabled(s.config) {
		t.Fatal("failed stop must not pretend that recording is off")
	}
	backend.failRequest = ""
	view.Click("记录候选诊断")
	if diagnosticEnabled(backend.config) {
		t.Fatal("switching off must stop recording immediately")
	}
}
