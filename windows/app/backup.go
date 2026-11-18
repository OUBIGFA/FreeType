package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/egoist/mygo/ui"
)

// backupParts 是一键导出能选的几类数据，顺序就是界面上的顺序。
// ID 和服务进程的 BackupPart 一样；fields 是这一类在备份文件里占的字段。
// 最后一类「同步账号和计划」由设置程序自己写进备份，服务进程不管。
var backupParts = []struct {
	ID, Title, Detail string
	fields            []string
}{
	{"settings", "设置", "设置页里的各个选项、中英文状态、悬浮状态栏的位置", []string{"config", "mode", "status_bar_position"}},
	{"dictionaries", "词库", "导入的词库和自定义主词库", []string{"dictionaries", "main_lexicon"}},
	{"quick_phrases", "快捷短语", "", []string{"quick_phrases"}},
	{"common_phrases", "常用语", "", []string{"common_phrases"}},
	{"learning", "学习记录", "学到的词、输入习惯、最近用过的表情", []string{"learning", "recent_emoji"}},
	{backupSyncPart, "同步账号和计划", "不含密码和密钥", []string{"sync_connection"}},
}

const backupSyncPart = "sync"

// chosenExportParts 是现在选中要导出的类别（没打开过选择框时是全部）。
func (s *settings) chosenExportParts() []string {
	chosen := []string{}
	for i, part := range backupParts {
		if i >= len(s.exportParts) || s.exportParts[i] {
			chosen = append(chosen, part.ID)
		}
	}
	return chosen
}

// exportedSummary 说明导出了什么：服务进程报的各类条数，加上设置程序自己写的同步账号。
func exportedSummary(response Response, parts []string) string {
	summary := ""
	if response.Report != nil {
		summary = response.Report.Note
	}
	for _, id := range parts {
		if id != backupSyncPart {
			continue
		}
		if summary == "没有输入法数据" {
			return "同步账号和计划"
		}
		return summary + "；含同步账号和计划"
	}
	return summary
}

// 全部设置备份放在「数据与同步」页开头；恢复快照会替换数据，先校验再让用户确认。
func (s *settings) backupSection(c *ui.Context) {
	section(c, "一键导入导出", func() {
		s.settingRow(c, "sync/全部设置和数据", func() {
			if ui.Button(c, "导入…").Label("导入全部设置").Clicked() {
				s.chooseSettingsBackup()
			}
			if ui.Button(c, "导出…").Label("导出全部设置").Clicked() {
				if len(s.exportParts) != len(backupParts) {
					s.exportParts = make([]bool, len(backupParts))
					for i := range s.exportParts {
						s.exportParts[i] = true
					}
				}
				s.exportChoosing = true
			}
		})
	})
	s.exportDialog(c)
	switch ui.AlertDialog(c, &s.confirmRestore, "导入全部设置？", s.restoreSummary+"\n导入会用备份里的数据替换本机对应的数据；备份里不含的数据保持不变。请先导出当前数据，保存后再导入。", "取消", "导入并替换") {
	case 0:
		s.restorePath, s.restoreSummary = "", ""
	case 1:
		path := s.restorePath
		s.restorePath, s.restoreSummary = "", ""
		s.importSettingsBackup(path)
	}
}

// exportDialog 让用户勾选要导出的数据，再选保存位置。
func (s *settings) exportDialog(c *ui.Context) {
	ui.Modal(c, &s.exportChoosing, func() {
		ui.Column(c).Gap(14).Width(420).MaxWidthPercent(100).Children(func() {
			ui.Text(c, "选择要导出的数据").FontSize(textSize(c, 17)).FontWeight(600)
			ui.Text(c, "没勾选的数据不会写进备份文件。导入这份备份时，只替换它包含的数据。").Wrap().TextColor(colors(c).muted)
			ui.Column(c).Gap(10).Children(func() {
				for i, part := range backupParts {
					ui.Column(c).Key("export-" + part.ID).Gap(2).Children(func() {
						checkbox(c, &s.exportParts[i], part.Title).Label("导出" + part.Title)
						if part.Detail != "" {
							ui.Text(c, part.Detail).FontSize(textSize(c, 12.5)).TextColor(colors(c).muted).Wrap().Padding(0, 0, 0, 26)
						}
					})
				}
			})
			none := len(s.chosenExportParts()) == 0
			if none {
				ui.Text(c, "请至少选择一项。").TextColor(c.Theme().Danger)
			}
			ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
				if ui.Button(c, "取消").Label("取消导出").Clicked() {
					s.exportChoosing = false
				}
				if ui.PrimaryButton(c, "导出…").Label("导出选中的数据").Disabled(none).Clicked() {
					s.exportChoosing = false
					s.export("export_settings", "导出全部设置", "FreeType 全部设置.json")
				}
			})
		})
	})
}

func (s *settings) chooseSettingsBackup() {
	s.background(func() func() {
		path := s.openFile("导入全部设置")
		if path == "" {
			return nil
		}
		response, err := s.backend.Call(op("inspect_settings_backup", "path", path))
		var connection *backupSyncConnection
		if err == nil {
			connection, err = readBackupConnection(path)
		}
		return func() {
			if err != nil {
				s.notice = "导入失败：" + err.Error()
				return
			}
			s.restorePath = path
			s.restoreSummary = "文件：" + filepath.Base(path)
			if connection != nil {
				s.restoreSummary += "\n包含同步账号和计划。导入后会清空密码并暂停同步，请重新填写密码。"
			}
			if response.Report != nil && response.Report.Note != "" {
				s.restoreSummary += "\n备份内容：" + response.Report.Note
			}
			s.confirmRestore = true
		}
	})
}

func (s *settings) importSettingsBackup(path string) {
	s.background(func() func() {
		// 与后台同步共用锁，避免恢复连接信息时仍向旧地址读写。
		if s.autoSync != nil {
			if !s.autoSync.gate.TryLock() {
				return func() { s.notice = "同步正在进行，请稍后再导入" }
			}
			defer s.autoSync.gate.Unlock()
		}
		connection, err := readBackupConnection(path)
		var restored syncSettings
		if err == nil && connection != nil {
			restored, err = loadSyncSettings(s.syncDataDir())
			if err == nil {
				data, _ := json.Marshal(connection)
				err = json.Unmarshal(data, &restored)
				// 账号可迁移，设备身份和凭据不能迁移；等用户确认新连接再启用。
				restored.Enabled, restored.WebDAVPassword, restored.S3SecretKey, restored.EncryptPassword = false, "", "", ""
			}
		}
		var response Response
		if err == nil {
			response, err = s.backend.Call(op("import_settings", "path", path))
		}
		var saveErr error
		if err == nil && connection != nil {
			saveErr = saveSyncSettings(s.syncDataDir(), restored)
			if saveErr == nil && s.autoSync != nil {
				s.autoSync.configure(restored)
			}
		}
		return func() {
			if err != nil {
				s.notice = "导入失败：" + err.Error()
				return
			}
			s.applyRestored(response)
			s.notice = "已导入备份"
			if connection != nil {
				if saveErr != nil {
					s.notice = "数据已导入，但同步账号保存失败：" + saveErr.Error()
					return
				}
				s.sync, s.syncLoaded = restored, true
				s.syncDevices, s.syncListed = nil, false
				s.syncMessageArea, s.syncMessage, s.syncFailed = "connection", "已导入账号和计划，请重新填写密码并测试连接，再启用同步", false
				s.notice += "；同步账号已恢复，请重新填写密码并启用同步"
			}
			if response.Report != nil && response.Report.Note != "" {
				s.notice += "（" + response.Report.Note + "）"
			}
		}
	})
}

// 白名单保证今后 syncSettings 增加私密字段时不会自动进入可携带备份。
// 不包含密码、Secret Key、设备标识及同步运行状态。
type backupSyncConnection struct {
	Provider        string `json:"provider"`
	WebDAVURL       string `json:"webdav_url"`
	WebDAVUser      string `json:"webdav_user"`
	S3Endpoint      string `json:"s3_endpoint"`
	S3Region        string `json:"s3_region"`
	S3Bucket        string `json:"s3_bucket"`
	S3Prefix        string `json:"s3_prefix"`
	S3AccessKey     string `json:"s3_access_key"`
	S3PathStyle     bool   `json:"s3_path_style"`
	Automatic       bool   `json:"automatic"`
	Schedule        string `json:"schedule"`
	IntervalMinutes int    `json:"interval_minutes"`
	DailyTime       string `json:"daily_time"`
	SyncOnStart     bool   `json:"sync_on_start"`
	DailyLimit      int    `json:"daily_limit"`
	RetryLimit      int    `json:"retry_limit"`
	KeepSnapshots   int    `json:"keep_snapshots"`
	// 云端备份有没有加密跟着账号走；备份密码是凭据，不带。
	Encrypt bool `json:"encrypt"`
}

func readBackupConnection(path string) (*backupSyncConnection, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var backup struct {
		Connection *backupSyncConnection `json:"sync_connection"`
	}
	if err := json.Unmarshal(bytes.TrimPrefix(data, []byte("\xef\xbb\xbf")), &backup); err != nil {
		return nil, err
	}
	if backup.Connection != nil {
		raw, _ := json.Marshal(backup.Connection)
		var c syncSettings
		if err := json.Unmarshal(raw, &c); err != nil {
			return nil, err
		}
		if c.Provider != "webdav" && c.Provider != "s3" {
			return nil, fmt.Errorf("备份中的同步服务无效")
		}
		if err := validateSyncPlan(c); err != nil {
			return nil, fmt.Errorf("备份中的同步计划无效：%w", err)
		}
	}
	return backup.Connection, nil
}

// exportSettingsBackup 把 parts 这几类数据导出到 path。
func (s *settings) exportSettingsBackup(path string, parts []string) (Response, error) {
	chosen := map[string]bool{}
	for _, id := range parts {
		chosen[id] = true
	}
	// 交给服务进程的类别。一类都没有时也要发空列表：不写这一项的意思是「全部」
	server := []string{}
	for _, part := range backupParts {
		if chosen[part.ID] && part.ID != backupSyncPart {
			server = append(server, part.ID)
		}
	}
	var connection syncSettings
	if chosen[backupSyncPart] {
		var err error
		if connection, err = loadSyncSettings(s.syncDataDir()); err != nil {
			return Response{}, err
		}
	}
	// 先在目标目录生成完整内容，所有步骤成功后才替换用户选择的文件。
	temp, err := os.CreateTemp(filepath.Dir(path), ".freetype-backup-*.tmp")
	if err != nil {
		return Response{}, err
	}
	name := temp.Name()
	defer os.Remove(name)
	if err := temp.Close(); err != nil {
		return Response{}, err
	}
	response, err := s.backend.Call(op("export_settings", "path", name, "parts", server))
	if err != nil {
		return response, err
	}
	data, err := os.ReadFile(name)
	if err != nil {
		return response, err
	}
	var document map[string]json.RawMessage
	if err := json.Unmarshal(data, &document); err != nil {
		return response, err
	}
	if document == nil {
		return response, fmt.Errorf("导出的设置备份为空")
	}
	// 没选的数据一个字段都不能在文件里。服务进程还是旧版本时（更新到一半）它会照旧导出全部，
	// 这里拦下来，不把用户没选的数据写出去。
	for _, part := range backupParts {
		if chosen[part.ID] {
			continue
		}
		for _, field := range part.fields {
			if _, present := document[field]; present {
				return response, fmt.Errorf("输入法服务还是旧版本，不能只导出选中的数据。请退出并重新打开 FreeType 后再试")
			}
		}
	}
	if chosen[backupSyncPart] {
		raw, err := json.Marshal(connection)
		if err != nil {
			return response, err
		}
		var portable backupSyncConnection
		if err := json.Unmarshal(raw, &portable); err != nil {
			return response, err
		}
		document["sync_connection"], err = json.Marshal(portable)
		if err != nil {
			return response, err
		}
	}
	data, err = json.MarshalIndent(document, "", "  ")
	if err != nil {
		return response, err
	}
	if err := os.WriteFile(name, append(data, '\n'), 0600); err != nil {
		return response, err
	}
	return response, os.Rename(name, path)
}
