package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"

	"github.com/egoist/mygo/ui"
)

const freeTypeTip = "0804:{6AEA9A65-33E4-4FFA-94E3-399C2ADC4CF1}{2C829DC6-FA03-477E-85D0-C64E1C64B209}"

func windowsDefaultTip(change bool) (bool, error) {
	// Fixed commands only: language, TIP and script never come from user text.
	// Find the language by the FreeType TIP instead of its tag: Windows 11 reports
	// Simplified Chinese as zh-Hans-CN, and older systems may use zh-CN or zh-Hans.
	// Errors go to stderr as UTF-8 so the settings page can show the real cause.
	script := `[Console]::OutputEncoding = [Text.Encoding]::UTF8; $ErrorActionPreference = 'Stop'; try { $tip = '` + freeTypeTip + `'; $list = Get-WinUserLanguageList; $zh = $list | Where-Object { $_.InputMethodTips -contains $tip } | Select-Object -First 1; `
	if change {
		script += `if ($null -eq $zh) { throw '没有在 Windows 语言的键盘列表中找到 FreeType；请先在系统设置中添加' }; [void]$zh.InputMethodTips.Remove($tip); $zh.InputMethodTips.Insert(0, $tip); Set-WinUserLanguageList $list -Force; Set-WinDefaultInputMethodOverride -InputTip $tip; `
	}
	script += `$current = Get-WinDefaultInputMethodOverride; if ($null -ne $current -and $current.InputMethodTip) { [int]($current.InputMethodTip -eq $tip) } else { [int]($null -ne $zh -and $zh.InputMethodTips[0] -eq $tip) } } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }`
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, filepath.Join(os.Getenv("SystemRoot"), "System32", "WindowsPowerShell", "v1.0", "powershell.exe"), "-NoProfile", "-NonInteractive", "-Command", script)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
	result, err := cmd.Output()
	if err != nil {
		var exit *exec.ExitError
		if errors.As(err, &exit) && len(bytes.TrimSpace(exit.Stderr)) > 0 {
			return false, fmt.Errorf("无法读取或设置 Windows 默认输入法：%s", strings.TrimSpace(string(exit.Stderr)))
		}
		return false, fmt.Errorf("无法读取或设置 Windows 默认输入法: %w", err)
	}
	return strings.TrimSpace(string(result)) == "1", nil
}

func (s *settings) refreshDefaultTip(change bool) {
	if s.defaultTip == nil || s.defaultTipBusy {
		return
	}
	s.defaultTipBusy = true
	s.background(func() func() {
		active, err := s.defaultTip(change)
		return func() {
			s.defaultTipBusy, s.defaultTipKnown, s.defaultTipActive = false, err == nil, active
			if change && err != nil {
				s.notice = err.Error()
			}
		}
	})
}

func (s *settings) defaultTipRow(c *ui.Context) {
	value := "未检测"
	if s.defaultTipBusy {
		value = "正在读取"
	} else if s.defaultTipKnown {
		if s.defaultTipActive {
			value = "已设为默认"
		} else {
			value = "未设为默认"
		}
	}
	s.settingRowValue(c, "general/默认输入法", value, func() {
		if ui.Button(c, "设为默认").Disabled(s.defaultTip == nil || s.defaultTipBusy || s.defaultTipActive).Clicked() {
			s.refreshDefaultTip(true)
		}
		if ui.Button(c, "系统设置").Disabled(s.openURL == nil).Clicked() {
			s.openLink("ms-settings:regionlanguage")
		}
	})
}

func (s *settings) neuralModelRow(c *ui.Context) {
	label := map[string]string{"bundled": "内置模型", "external": "导入模型"}[s.status.NeuralModel]
	if label == "" {
		label = "未加载"
	}
	s.settingRowValue(c, "features/整句增强模型", label, func() {
		if ui.Button(c, "导入模型").Disabled(!s.status.Ready || s.openModel == nil || s.modelBusy).Clicked() {
			s.changeNeuralModel(true)
		}
		if s.status.NeuralModel == "external" && ui.Button(c, "恢复内置模型").Disabled(s.modelBusy).Clicked() {
			s.confirmModelReset = true
		}
	})
	if ui.AlertDialog(c, &s.confirmModelReset, "移除外部模型", "移除 FreeType 保存的副本，原文件不会改变", "取消", "移除") == 1 {
		s.changeNeuralModel(false)
	}
}

func (s *settings) changeNeuralModel(importing bool) {
	s.modelBusy = true
	s.background(func() func() {
		var path any
		if importing {
			file := s.openModel("导入整句增强模型")
			if file == "" {
				return func() { s.modelBusy = false }
			}
			path = file
		}
		response, err := s.backend.Call(op("set_neural_model", "path", path))
		return func() {
			s.modelBusy = false
			if err != nil {
				s.notice = err.Error()
				return
			}
			if response.Status != nil {
				s.status = *response.Status
			}
		}
	})
}

// The export is allowlisted. Paths, manually configured font names, app rules,
// remote accounts, learning, statistics, and raw log lines are never included.
func diagnosticReport(version string, status Status, config Config, resources string) map[string]any {
	data := map[string]string{}
	for file, key := range map[string]string{"MANIFEST.json": "lexicon", "neural-model.json": "neural_model"} {
		var record struct {
			Version string `json:"version"`
		}
		if bytes, err := os.ReadFile(filepath.Join(resources, "data", file)); err == nil && json.Unmarshal(bytes, &record) == nil && len(record.Version) <= 32 && strings.Trim(record.Version, "0123456789.-") == "" {
			data[key] = record.Version
		}
	}
	return map[string]any{
		"version": version, "os": runtime.GOOS, "arch": runtime.GOARCH, "pipe": status.Protocol,
		"service_ready": status.Ready, "service_has_error": status.Error != "", "data_versions": data,
		"config": map[string]any{"horizontal": config.Horizontal, "page_size": config.PageSize, "font_size": config.FontSize,
			"traditional": config.Traditional, "shuangpin": config.Shuangpin, "typing_stats": config.TypingStats,
			"predict_next": config.PredictNext, "predict_double_space": config.PredictDoubleSpace, "model_source": status.NeuralModel},
		"recent_error_counts": diagnosticErrors(status.LogDir),
	}
}

func diagnosticErrors(directory string) map[string]int {
	counts := map[string]int{}
	if directory == "" {
		return counts
	}
	for _, name := range []string{"server.log", "tsf.log"} {
		file, err := os.Open(filepath.Join(directory, name))
		if err != nil {
			continue
		}
		if info, err := file.Stat(); err == nil && info.Size() > 65536 {
			_, _ = file.Seek(-65536, io.SeekEnd)
		}
		data, _ := io.ReadAll(io.LimitReader(file, 65536))
		_ = file.Close()
		for _, marker := range []string{"词库载入失败", "候选窗绘制失败", "更新文档失败", "服务意外退出"} {
			if n := strings.Count(string(data), marker); n > 0 {
				counts[marker] += n
			}
		}
	}
	return counts
}

func (s *settings) exportDiagnostics() {
	if s.saveFile == nil {
		return
	}
	status, config := s.status, s.config
	s.background(func() func() {
		path := s.saveFile("导出诊断报告", "FreeType-diagnostics.json")
		if path == "" {
			return nil
		}
		data, err := json.MarshalIndent(diagnosticReport(s.version, status, config, s.resources), "", "  ")
		if err == nil {
			err = os.WriteFile(path, data, 0o600)
		}
		return func() {
			if err != nil {
				s.notice = "导出诊断失败：" + err.Error()
			} else {
				s.notice = "已导出脱敏诊断报告"
			}
		}
	})
}
