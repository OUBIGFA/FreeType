package main

import (
	"fmt"
	"github.com/egoist/mygo/ui"
)

func (s *settings) candidateDiagnosticsSection(c *ui.Context) {
	if !s.loaded {
		return
	}
	section(c, "候选诊断", func() {
		enabled := s.config.CandidateDiagnostics
		if s.settingSwitch(c, "about/候选诊断", false, &enabled) {
			if enabled {
				s.confirmDiagnostics = true
			} else {
				s.config.CandidateDiagnostics = false
				s.saveConfig()
			}
		}
		message := "点击刷新查看记录大小；关闭后保留已有记录"
		if status := s.diagnosticsStatus; status != nil {
			message = fmt.Sprintf("记录 %.1f MB；跳过 %d 条", float64(status.Bytes)/1000000, status.Dropped)
			if status.Error != "" {
				message += "；" + status.Error
			}
		}
		if s.diagnosticsError != "" {
			message = s.diagnosticsError
		}
		row(c, "本机记录", message, func() {
			if ui.Button(c, "刷新状态").Disabled(s.diagnosticsBusy).Clicked() {
				s.runCandidateDiagnostics("get_candidate_diagnostics", false)
			}
			if ui.Button(c, "清空记录").Disabled(s.diagnosticsBusy).Clicked() {
				s.confirmDiagnosticsClear = true
			}
		})
		row(c, "导出内容", "记录包含正文；请自行检查后再分享", func() {
			if ui.Button(c, "导出记录").Disabled(s.diagnosticsBusy || s.saveFile == nil).Clicked() {
				s.runCandidateDiagnostics("export_candidate_diagnostics", false)
			}
			if ui.Button(c, "导出评测样本").Disabled(s.diagnosticsBusy || s.saveFile == nil).Clicked() {
				s.runCandidateDiagnostics("export_candidate_diagnostics", true)
			}
		})
	})
	ui.Modal(c, &s.confirmDiagnostics, func() {
		ui.Column(c).Gap(16).Width(440).MaxWidthPercent(100).Children(func() {
			ui.Text(c, "开启候选诊断").FontSize(textSize(c, 17)).FontWeight(600)
			ui.Text(c, "会记录正文、拼音、上下文、候选和应用名称；仅保存在本机，普通诊断导出不包含这些内容").Wrap()
			ui.Text(c, "未标记为密码框的输入区也可能被记录；排查后请关闭并清空").Wrap().TextColor(colors(c).muted)
			ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
				if ui.Button(c, "取消").Clicked() {
					s.confirmDiagnostics = false
				}
				if ui.PrimaryButton(c, "确认开启").Clicked() {
					s.confirmDiagnostics = false
					s.config.CandidateDiagnostics = true
					s.saveConfig()
				}
			})
		})
	})
	if ui.AlertDialog(c, &s.confirmDiagnosticsClear, "清空候选诊断", "清空这台电脑保存的候选诊断记录；无法恢复", "取消", "确认清空") == 1 {
		s.runCandidateDiagnostics("clear_candidate_diagnostics", false)
	}
}

func (s *settings) runCandidateDiagnostics(operation string, evaluation bool) {
	if s.diagnosticsBusy {
		return
	}
	s.diagnosticsBusy = true
	s.background(func() func() {
		request := op(operation)
		if operation == "export_candidate_diagnostics" {
			name := "FreeType-candidate-diagnostics.jsonl"
			if evaluation {
				name = "FreeType-candidate-evaluation.tsv"
			}
			path := s.saveFile("导出候选诊断", name)
			if path == "" {
				return func() { s.diagnosticsBusy = false }
			}
			request["path"], request["evaluation"] = path, evaluation
		}
		response, err := s.backend.Call(request)
		if err == nil && !response.OK {
			err = fmt.Errorf("%s", response.Error)
		}
		return func() {
			s.diagnosticsBusy = false
			if err != nil {
				s.diagnosticsError = err.Error()
				return
			}
			s.diagnosticsStatus, s.diagnosticsError = response.CandidateDiagnostics, ""
			if operation == "export_candidate_diagnostics" {
				s.notice = "已导出候选诊断"
			}
			if operation == "clear_candidate_diagnostics" {
				s.notice = "已清空候选诊断记录"
			}
		}
	})
}
