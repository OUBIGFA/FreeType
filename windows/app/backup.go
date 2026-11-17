package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/egoist/mygo/ui"
)

// 全部设置备份放在「数据与同步」页开头；恢复快照会替换数据，先校验再让用户确认。
func (s *settings) backupSection(c *ui.Context) {
	section(c, "一键导入导出", func() {
		s.settingRow(c, "sync/全部设置和数据", func() {
			if ui.Button(c, "导入…").Label("导入全部设置").Clicked() {
				s.chooseSettingsBackup()
			}
			if ui.Button(c, "导出…").Label("导出全部设置").Clicked() {
				s.export("export_settings", "导出全部设置", "FreeType 全部设置.json")
			}
		})
		ui.Text(c, "包含同步账号和计划，不含密码或密钥；导入后需重新填写密码。").Wrap().TextColor(colors(c).muted).Padding(0, 18, 14, 18)
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
				s.restoreSummary += "\n" + response.Report.Note
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
				restored.Enabled, restored.WebDAVPassword, restored.S3SecretKey = false, "", ""
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
			s.notice = "已导入全部设置和数据"
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

func (s *settings) exportSettingsBackup(path string) (Response, error) {
	connection, err := loadSyncSettings(s.syncDataDir())
	if err != nil {
		return Response{}, err
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
	response, err := s.backend.Call(op("export_settings", "path", name))
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
	data, err = json.MarshalIndent(document, "", "  ")
	if err != nil {
		return response, err
	}
	if err := os.WriteFile(name, append(data, '\n'), 0600); err != nil {
		return response, err
	}
	return response, os.Rename(name, path)
}
