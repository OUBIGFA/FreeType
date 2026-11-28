package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// 默认设置夹具由 windows/protocol/tests/config_contract.rs 按服务进程的 Config::default() 核对。
func loadConfigFixture(t *testing.T) map[string]any {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "protocol", "tests", "fixtures", "config-default.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]any
	if err := json.Unmarshal(data, &fields); err != nil {
		t.Fatal(err)
	}
	return fields
}

func configFields(t *testing.T, config Config) map[string]any {
	t.Helper()
	data, err := json.Marshal(config)
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]any
	if err := json.Unmarshal(data, &fields); err != nil {
		t.Fatal(err)
	}
	return fields
}

func diffFields(t *testing.T, want, got map[string]any) {
	t.Helper()
	for key, value := range want {
		if other, ok := got[key]; !ok {
			t.Errorf("Go Config 缺少字段 %q；保存设置时这一项会回到默认值", key)
		} else if !reflect.DeepEqual(value, other) {
			t.Errorf("字段 %q：服务进程为 %v，设置程序为 %v", key, value, other)
		}
	}
	for key := range got {
		if _, ok := want[key]; !ok {
			t.Errorf("Go Config 多出服务进程没有的字段 %q", key)
		}
	}
}

// 设置程序整份发回 Config：读入服务进程的设置再写出，必须不丢字段、不改取值。
func TestConfigRoundTripKeepsEveryServerField(t *testing.T) {
	want := loadConfigFixture(t)
	data, _ := json.Marshal(want)
	var config Config
	if err := json.Unmarshal(data, &config); err != nil {
		t.Fatal(err)
	}
	diffFields(t, want, configFields(t, config))
}

// 连不上服务进程时界面先按 defaultConfig 画，取值必须与服务进程的默认值一致。
func TestDefaultConfigMatchesServerDefaults(t *testing.T) {
	diffFields(t, loadConfigFixture(t), configFields(t, defaultConfig()))
}
