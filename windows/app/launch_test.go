package main

import (
	"encoding/json"
	"os"
	"slices"
	"testing"
)

func TestSettingsLaunchRespectsInstallPreferenceAndExplicitLaunches(t *testing.T) {
	for _, sample := range []struct {
		name string
		args []string
		on   bool
		want bool
	}{
		{"manual-default", nil, false, true},
		{"manual-enabled", nil, true, true},
		{"background-default", []string{"--background"}, false, false},
		{"background-enabled", []string{"--background"}, true, false},
		{"after-install-default", []string{"--after-install"}, false, false},
		{"after-install-enabled", []string{"--after-install"}, true, true},
		{"installer-background-default", []string{"--background", "--after-install"}, false, false},
		{"installer-background-enabled", []string{"--background", "--after-install"}, true, true},
	} {
		t.Run(sample.name, func(t *testing.T) {
			backend := &fakeBackend{config: defaultConfig()}
			backend.config.ShowSettingsAfterUpdate = sample.on
			got, err := shouldOpenSettings(sample.args, backend)
			if err != nil || got != sample.want {
				t.Fatalf("显示设置页 = %v，错误 = %v；应为 %v", got, err, sample.want)
			}
			if !slices.Contains(sample.args, "--after-install") && len(backend.calls) != 0 {
				t.Fatal("主动打开设置和后台启动不应依赖服务返回安装选项")
			}
		})
	}
}

func TestInstallerLaunchTreatsOldConfigAsOffAndReportsReadFailures(t *testing.T) {
	var old Config
	if err := json.Unmarshal([]byte(`{"tray_icon":true}`), &old); err != nil {
		t.Fatal(err)
	}
	backend := &fakeBackend{config: old}
	if show, err := shouldOpenSettings([]string{"--after-install"}, backend); err != nil || show {
		t.Fatalf("旧设置应默认不弹出设置页：show=%v err=%v", show, err)
	}
	backend.failRequest = "get_config"
	if show, err := shouldOpenSettings([]string{"--after-install"}, backend); err == nil || show {
		t.Fatalf("读取失败应报告错误且不弹出设置页：show=%v err=%v", show, err)
	}
}

func TestInstallerFinishLaunchForwardsMarkerThenClearsEnvironment(t *testing.T) {
	for _, args := range [][]string{nil, {"--after-install"}} {
		t.Setenv("FREETYPE_INSTALL_COMPLETED", "1")
		got, err := consumeInstallerLaunch(args)
		if err != nil || !slices.Equal(got, []string{"--after-install"}) {
			t.Fatalf("安装器启动参数 = %v，错误 = %v", got, err)
		}
		if _, exists := os.LookupEnv("FREETYPE_INSTALL_COMPLETED"); exists {
			t.Fatal("安装标记不能传给从设置打开的其他程序")
		}
		manual, err := consumeInstallerLaunch(nil)
		if err != nil || len(manual) != 0 {
			t.Fatalf("后续主动启动不应继续携带安装标记：%v %v", manual, err)
		}
	}
}
