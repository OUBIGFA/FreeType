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
	{"settings", "设置", "输入选项、中英文状态和悬浮状态栏位置", []string{"config", "mode", "status_bar_position"}},
	{"dictionaries", "词库", "导入的词库和自定义主词库", []string{"dictionaries", "main_lexicon"}},
	{"quick_phrases", "快捷短语", "", []string{"quick_phrases"}},
	{"common_phrases", "常用语", "", []string{"common_phrases"}},
	{"learning", "学习记录", "学到的词、选词次数、搭配和最近使用的表情", []string{"learning", "recent_emoji"}},
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
			if ui.Button(c, "导入").Label("导入全部设置").Clicked() {
				s.chooseSettingsBackup()
			}
			if ui.Button(c, "导出").Label("导出全部设置").Clicked() {
				if len(s.exportParts) != len(backupParts) {
					s.exportParts = make([]bool, len(backupParts))
					for i := range s.exportParts {
						s.exportParts[i] = true
					}
				}
				s.exportPassword = ""
				s.exportChoosing = true
			}
		})
	})
	section(c, "恢复设置", func() {
		s.settingRow(c, "sync/恢复出厂设置", func() {
			if ui.Button(c, "恢复出厂设置").Disabled(!s.loaded || s.syncBusy != "" || s.autoSyncStatus.Busy).Clicked() {
				s.confirmFactoryReset = true
			}
		})
	})
	s.factoryResetDialog(c)
	s.exportDialog(c)
	s.unlockDialog(c)
	switch ui.AlertDialog(c, &s.confirmRestore, "导入全部设置", s.restoreSummary+"\n用备份替换对应数据；未备份的数据保持不变；请先导出当前数据", "取消", "导入并替换") {
	case 0:
		s.restorePath, s.restoreSummary, s.restorePassword = "", "", ""
	case 1:
		path := s.restorePath
		s.restorePath, s.restoreSummary = "", ""
		s.importSettingsBackup(path)
	}
}

// factoryResetDialog 分开列出会恢复默认和保持不变的内容，用户确认后才恢复。
func (s *settings) factoryResetDialog(c *ui.Context) {
	ui.Modal(c, &s.confirmFactoryReset, func() {
		ui.Column(c).Gap(16).Width(440).MaxWidthPercent(100).Children(func() {
			ui.Column(c).Gap(8).Children(func() {
				ui.Text(c, "恢复出厂设置").FontSize(textSize(c, fontHeading)).FontWeight(600)
				ui.Text(c, "恢复后无法撤销；如需留存当前设置，请先导出全部设置").Wrap().TextColor(colors(c).muted)
			})
			factoryResetScope(c, "恢复为默认", []string{
				"输入、候选窗、按键和外观等全部选项",
				"自定义主词库切回内置词库；原文件保留",
				"同步设置；清除本机保存的同步账号、密码和计划",
			})
			factoryResetScope(c, "保持不变", []string{
				"词库、快捷短语、常用语、学习记录和输入统计",
				"云端备份",
			})
			ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
				if ui.Button(c, "取消").Clicked() {
					s.confirmFactoryReset = false
				}
				if ui.PrimaryButton(c, "确认恢复").Clicked() {
					s.confirmFactoryReset = false
					s.resetSettings()
				}
			})
		})
	})
}

// factoryResetScope 是确认框里的一组条目：小标题加圆点列表；弹窗里不再套底色框，组间靠留白区分。
func factoryResetScope(c *ui.Context, title string, items []string) {
	p := colors(c)
	ui.Column(c).Key("reset-" + title).Gap(space2).Children(func() {
		ui.Text(c, title).FontSize(textSize(c, fontCaption)).FontWeight(600)
		for _, item := range items {
			ui.Row(c).Key(item).Gap(8).AlignItems(ui.Start).Children(func() {
				ui.Row(c).Width(4).Height(4).Radius(2).Background(p.muted).Margin(textSize(c, 8), 0, 0, 0)
				ui.Text(c, item).FontSize(textSize(c, fontCaption)).TextColor(p.muted).Wrap().Shrink(1)
			})
		}
	})
}

// exportDialog 让用户勾选要导出的数据（并可选设置加密密码），再选保存位置。
func (s *settings) exportDialog(c *ui.Context) {
	ui.Modal(c, &s.exportChoosing, func() {
		ui.Column(c).Gap(16).Width(420).MaxWidthPercent(100).Children(func() {
			ui.Text(c, "选择要导出的数据").FontSize(textSize(c, fontHeading)).FontWeight(600)
			ui.Text(c, "只备份所选数据；导入时只替换备份中的数据").Wrap().TextColor(colors(c).muted)
			ui.Column(c).Gap(12).Children(func() {
				for i, part := range backupParts {
					ui.Column(c).Key("export-" + part.ID).Gap(4).Children(func() {
						checkbox(c, &s.exportParts[i], part.Title).Label("导出" + part.Title)
						if part.Detail != "" {
							ui.Text(c, part.Detail).FontSize(textSize(c, fontCaption)).TextColor(colors(c).muted).Wrap().Padding(0, 0, 0, 28)
						}
					})
				}
			})
			ui.Field(c, "备份密码 可选", func() {
				ui.TextInput(c, &s.exportPassword).Password().Placeholder("留空则不加密").Label("导出备份密码").FillWidth()
			}).FillWidth()
			none := len(s.chosenExportParts()) == 0
			if none {
				ui.Text(c, "请至少选择一项").TextColor(c.Theme().Danger)
			}
			ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
				if ui.Button(c, "取消").Label("取消导出").Clicked() {
					s.exportChoosing, s.exportPassword = false, ""
				}
				if ui.PrimaryButton(c, "导出").Label("导出选中的数据").Disabled(none).Clicked() {
					s.exportChoosing = false
					s.export("export_settings", "导出全部设置", "FreeType 全部设置.json")
				}
			})
		})
	})
}

// unlockDialog 在导入已加密的备份文件时提示用户输入密码。
func (s *settings) unlockDialog(c *ui.Context) {
	ui.Modal(c, &s.unlockingRestore, func() {
		ui.Column(c).Gap(16).Width(400).MaxWidthPercent(100).Children(func() {
			ui.Text(c, "输入备份密码").FontSize(textSize(c, fontHeading)).FontWeight(600)
			ui.Text(c, "备份文件 "+filepath.Base(s.lockedBackupPath)+" 已加密；请输入导出时设置的备份密码").Wrap().TextColor(colors(c).muted)
			ui.Field(c, "备份密码", func() {
				ui.TextInput(c, &s.restorePassword).Password().Placeholder("备份密码").Label("导入备份密码").FillWidth()
			}).FillWidth()
			if s.unlockError != "" {
				ui.Text(c, s.unlockError).Wrap().TextColor(c.Theme().Danger)
			}
			ui.Row(c).Gap(8).Justify(ui.End).Children(func() {
				if ui.Button(c, "取消").Label("取消解锁备份").Clicked() {
					s.unlockingRestore = false
					s.lockedBackupPath, s.restorePassword, s.unlockError = "", "", ""
				}
				if ui.PrimaryButton(c, "继续").Label("解锁备份").Clicked() {
					if s.restorePassword == "" {
						s.unlockError = "请输入备份密码"
						return
					}
					path, password, learning := s.lockedBackupPath, s.restorePassword, s.unlockLearning
					s.background(func() func() {
						if learning {
							return s.importLearningFromFile(path, password)
						}
						return s.inspectSettingsBackupFile(path, password)
					})
				}
			})
		})
	})
}

func checkEncryptedBackupFile(path string) (bool, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return false, err
	}
	return isEncryptedBackup(data), nil
}

// withPlainBackupFile 将可能加密的备份还原为临时明文文件供服务进程读取，调用结束后立即删除临时文件。
func withPlainBackupFile(path, password string, fn func(plainPath string, rawPlain []byte) error) error {
	if password == "" {
		raw, _ := os.ReadFile(path)
		return fn(path, raw)
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	plain, err := openLocalBackup(raw, password)
	if err != nil {
		return err
	}
	temp, err := os.CreateTemp(filepath.Dir(path), ".freetype-open-*.tmp")
	if err != nil {
		temp, err = os.CreateTemp("", ".freetype-open-*.tmp")
		if err != nil {
			return err
		}
	}
	name := temp.Name()
	defer os.Remove(name)
	if err := temp.Close(); err != nil {
		return err
	}
	if err := os.WriteFile(name, plain, 0600); err != nil {
		return err
	}
	return fn(name, plain)
}

func (s *settings) chooseSettingsBackup() {
	s.background(func() func() {
		path := s.openFile("导入全部设置")
		if path == "" {
			return nil
		}
		if locked, _ := checkEncryptedBackupFile(path); locked {
			return func() {
				s.lockedBackupPath = path
				s.restorePassword = ""
				s.unlockError = ""
				s.unlockLearning = false
				s.unlockingRestore = true
			}
		}
		return s.inspectSettingsBackupFile(path, "")
	})
}

func (s *settings) inspectSettingsBackupFile(path, password string) func() {
	var (
		response   Response
		connection *backupSyncConnection
	)
	err := withPlainBackupFile(path, password, func(plainPath string, rawPlain []byte) error {
		var inspectErr error
		response, inspectErr = s.backend.Call(op("inspect_settings_backup", "path", plainPath))
		if inspectErr != nil {
			return inspectErr
		}
		if len(rawPlain) > 0 {
			connection, inspectErr = readBackupConnectionBytes(rawPlain)
		} else {
			connection, inspectErr = readBackupConnection(plainPath)
		}
		return inspectErr
	})
	return func() {
		if err != nil {
			if password != "" {
				s.unlockError = err.Error()
				s.unlockingRestore = true
				return
			}
			s.notice = "导入失败：" + err.Error()
			return
		}
		s.unlockingRestore = false
		s.lockedBackupPath, s.unlockError = "", ""
		s.restorePath = path
		s.restorePassword = password
		s.restoreSummary = "文件：" + filepath.Base(path)
		if connection != nil {
			s.restoreSummary += "\n包含同步账号和计划；导入后会清空密码并暂停同步，请重新填写密码"
		}
		if response.Report != nil && response.Report.Note != "" {
			s.restoreSummary += "\n备份内容：" + response.Report.Note
		}
		s.confirmRestore = true
	}
}

func (s *settings) importLearningFromFile(path, password string) func() {
	var resp Response
	err := withPlainBackupFile(path, password, func(plainPath string, _ []byte) error {
		var callErr error
		resp, callErr = s.backend.Call(op("import_learning", "path", plainPath))
		return callErr
	})
	return func() {
		if err != nil {
			if password != "" {
				s.unlockError = err.Error()
				s.unlockingRestore = true
				return
			}
			s.notice = "导入失败：" + err.Error()
			return
		}
		s.unlockingRestore = false
		s.lockedBackupPath, s.restorePassword, s.unlockError = "", "", ""
		s.words = resp.UserWords
		s.wordsLoaded = true
		s.notice, _ = describe(resp.Report, "学习记录已合并")
	}
}

func (s *settings) importSettingsBackup(path string) {
	password := s.restorePassword
	s.restorePassword = ""
	s.background(func() func() {
		// 与后台同步共用锁，避免恢复连接信息时仍向旧地址读写。
		if s.autoSync != nil {
			if !s.autoSync.gate.TryLock() {
				return func() { s.notice = "同步正在进行，请稍后再导入" }
			}
			defer s.autoSync.gate.Unlock()
		}
		var (
			connection *backupSyncConnection
			restored   syncSettings
			response   Response
			saveErr    error
		)
		err := withPlainBackupFile(path, password, func(plainPath string, rawPlain []byte) error {
			var stepErr error
			if len(rawPlain) > 0 {
				connection, stepErr = readBackupConnectionBytes(rawPlain)
			} else {
				connection, stepErr = readBackupConnection(plainPath)
			}
			if stepErr == nil && connection != nil {
				restored, stepErr = loadSyncSettings(s.syncDataDir())
				if stepErr == nil {
					data, _ := json.Marshal(connection)
					stepErr = json.Unmarshal(data, &restored)
					// 账号可迁移，设备身份和凭据不能迁移；等用户确认新连接再启用。
					restored.Enabled, restored.WebDAVPassword, restored.S3SecretKey, restored.EncryptPassword = false, "", "", ""
				}
			}
			if stepErr == nil {
				response, stepErr = s.backend.Call(op("import_settings", "path", plainPath))
			}
			return stepErr
		})
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
				s.notice += "；" + response.Report.Note
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
	return readBackupConnectionBytes(data)
}

func readBackupConnectionBytes(data []byte) (*backupSyncConnection, error) {
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

// exportSettingsBackup 把 parts 这几类数据导出到 path；password 非空时用口令加密整份文件。
func (s *settings) exportSettingsBackup(path string, parts []string, password string) (Response, error) {
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
				return response, fmt.Errorf("当前服务不支持部分导出；请退出并重新打开 FreeType 后重试")
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
	var compactLearning bytes.Buffer
	if rawLearning, ok := document["learning"]; ok && len(rawLearning) > 0 && string(rawLearning) != "null" {
		if err := json.Compact(&compactLearning, rawLearning); err != nil {
			return response, err
		}
		document["learning"] = json.RawMessage("null")
	}
	data, err = json.MarshalIndent(document, "", "  ")
	if err != nil {
		return response, err
	}
	if compactLearning.Len() > 0 {
		data = bytes.Replace(
			data,
			[]byte("\n  \"learning\": null"),
			append([]byte("\n  \"learning\": "), compactLearning.Bytes()...),
			1,
		)
	}
	data = append(data, '\n')
	if password != "" {
		data, err = sealBackup(data, password)
		if err != nil {
			return response, err
		}
		data = append(data, '\n')
	}
	if err := os.WriteFile(name, data, 0600); err != nil {
		return response, err
	}
	return response, os.Rename(name, path)
}
