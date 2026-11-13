package main

import (
	"path/filepath"

	"github.com/egoist/mygo/ui"
)

// 全部设置备份放在常规页末尾；恢复快照会替换数据，先校验再让用户确认。
func (s *settings) backupSection(c *ui.Context) {
	section(c, "一键导入、导出", func() {
		s.settingRow(c, "general/全部设置和数据", func() {
			if ui.Button(c, "导入…").Label("导入全部设置").Clicked() {
				s.chooseSettingsBackup()
			}
			if ui.Button(c, "导出…").Label("导出全部设置").Clicked() {
				s.export("export_settings", "导出全部设置", "FreeType 全部设置.json")
			}
		})
	})
	switch ui.AlertDialog(c, &s.confirmRestore, "导入全部设置？", s.restoreSummary+"\n将用备份替换当前设置、词库、快捷短语、常用语和学习记录。请先导出当前数据，保存后再导入。", "取消", "导入并替换") {
	case 0:
		s.restorePath, s.restoreSummary = "", ""
	case 1:
		path := s.restorePath
		s.restorePath, s.restoreSummary = "", ""
		s.importSettingsBackup(path)
	}
}

func (s *settings) chooseSettingsBackup() {
	s.background(func() func() {
		path := s.openFile("导入全部设置")
		if path == "" {
			return nil
		}
		response, err := s.backend.Call(op("inspect_settings_backup", "path", path))
		return func() {
			if err != nil {
				s.notice = "导入失败：" + err.Error()
				return
			}
			s.restorePath = path
			s.restoreSummary = "文件：" + filepath.Base(path)
			if response.Report != nil && response.Report.Note != "" {
				s.restoreSummary += "\n" + response.Report.Note
			}
			s.confirmRestore = true
		}
	})
}

func (s *settings) importSettingsBackup(path string) {
	s.background(func() func() {
		response, err := s.backend.Call(op("import_settings", "path", path))
		return func() {
			if err != nil {
				s.notice = "导入失败：" + err.Error()
				return
			}
			s.config = *response.Config
			s.loaded = true
			s.status = *response.Status
			s.statusErr = ""
			s.dicts, s.phrases, s.common = response.Dictionaries, response.QuickPhrases, response.CommonPhrases
			s.words, s.wordsLoaded = response.UserWords, true
			s.report, s.reportErrors = "", nil
			if s.onTray != nil {
				s.onTray(s.config.TrayIcon)
			}
			if s.onAutomaticUpdate != nil {
				s.onAutomaticUpdate(s.config.AutoUpdate)
			}
			s.notice = "已导入全部设置和数据"
			if response.Report != nil && response.Report.Note != "" {
				s.notice += "（" + response.Report.Note + "）"
			}
		}
	})
}
