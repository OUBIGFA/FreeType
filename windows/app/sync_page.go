package main

import (
	"fmt"
	"path/filepath"
	"time"

	"github.com/egoist/mygo/ui"
)

// syncPage 是「数据与同步」页：整份数据的导入导出，以及上传到自己的网盘、在别的电脑上合并恢复。
func (s *settings) syncPage(c *ui.Context) {
	pageHeader(c, "数据与同步", "备份全部设置和数据，或通过你自己的网盘在几台电脑之间同步")
	s.backupSection(c)
	if !s.loaded {
		return
	}
	s.loadSync()
	busy := s.syncBusy != "" || s.autoSyncStatus.Busy
	section(c, "云端同步", func() {
		if s.settingSwitch(c, "sync/启用同步", false, &s.sync.Enabled) {
			s.saveSync()
		}
		if !s.sync.Enabled && s.syncMessageArea != "connection" && !s.showSetting("sync/同步服务") && !s.showSetting("sync/上传与恢复") && !s.showSetting("sync/自动同步") && !s.showSetting("sync/同步计划") {
			return
		}
		s.settingBlock(c, "sync/同步服务", func() {
			ui.Column(c).Padding(16, 18).Gap(12).Children(func() {
				provider := 0
				if s.sync.Provider == "s3" {
					provider = 1
				}
				if ui.Segmented(c, &provider, "WebDAV", "S3 兼容").AlignSelf(ui.Start).Label("同步服务").Changed() {
					s.sync.Provider = []string{"webdav", "s3"}[provider]
				}
				ui.Column(c).Children(func() {
					ui.Form(c, func() {
						if s.sync.Provider == "s3" {
							ui.Field(c, "服务地址", func() {
								ui.TextInput(c, &s.sync.S3Endpoint).Placeholder("https://s3.amazonaws.com").Width(360)
							}).Description("AWS S3、MinIO、Cloudflare R2、Backblaze B2、阿里云 OSS 等 S3 兼容服务的地址。")
							ui.Field(c, "区域", func() { ui.TextInput(c, &s.sync.S3Region).Placeholder("us-east-1").Width(200) })
							ui.Field(c, "存储桶", func() { ui.TextInput(c, &s.sync.S3Bucket).Width(240) })
							ui.Field(c, "目录", func() { ui.TextInput(c, &s.sync.S3Prefix).Placeholder("FreeType").Width(240) }).
								Description("统一保存在此目录下的 FreeType 文件夹中；留空则使用 FreeType。")
							ui.Field(c, "Access Key", func() { ui.TextInput(c, &s.sync.S3AccessKey).Width(360) })
							ui.Field(c, "Secret Key", func() { ui.TextInput(c, &s.sync.S3SecretKey).Password().Width(360) })
						} else {
							ui.Field(c, "地址", func() {
								ui.TextInput(c, &s.sync.WebDAVURL).Placeholder("https://dav.jianguoyun.com/dav/FreeType").Width(360)
							}).Description("自动使用地址下的 FreeType 文件夹；已以 FreeType 结尾时直接使用。坚果云需填写应用密码。")
							ui.Field(c, "账号", func() { ui.TextInput(c, &s.sync.WebDAVUser).Width(280) })
							ui.Field(c, "密码", func() { ui.TextInput(c, &s.sync.WebDAVPassword).Password().Width(280) })
						}
					})
				})
				if s.sync.Provider == "s3" {
					checkbox(c, &s.sync.S3PathStyle, "路径风格地址（MinIO 等自建服务要勾选；AWS、R2 可以不勾）")
				}
				ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
					if ui.PrimaryButton(c, "保存并测试连接").Disabled(busy).Clicked() {
						s.testSync()
					}
					ui.Column(c).Grow(1).Shrink(1).Children(func() { s.syncFeedback(c, "connection") })
				})
			})
		})
		if !s.sync.Enabled && !s.showSetting("sync/上传与恢复") && !s.showSetting("sync/自动同步") && !s.showSetting("sync/同步计划") {
			return
		}
		if s.settingSwitch(c, "sync/自动同步", false, &s.sync.Automatic) {
			s.saveSync()
		}
		s.settingBlock(c, "sync/同步计划", func() {
			ui.Column(c).Padding(14, 18).Gap(12).Children(func() {
				s.syncStatusSummary(c)
				ui.Form(c, func() {
					ui.Field(c, "同步方式", func() {
						mode := 0
						if s.sync.Schedule == "daily" {
							mode = 1
						}
						// 表单内容列默认拉伸子控件；分段按钮按标签宽度显示。
						if ui.Segmented(c, &mode, "按间隔", "每天定时").AlignSelf(ui.Start).Label("同步方式").Changed() {
							s.sync.Schedule = []string{"interval", "daily"}[mode]
						}
					})
					if s.sync.Schedule == "daily" {
						ui.Field(c, "每天时间", func() { ui.TextInput(c, &s.sync.DailyTime).Label("每天同步时间").Placeholder("21:00").Width(160) }).Description("按本机时间，24 小时制。")
					} else {
						ui.Field(c, "间隔（分钟）", func() {
							value := float64(s.sync.IntervalMinutes)
							if ui.NumberInput(c, &value, 1, 1440, 1).Label("同步间隔").Width(160).Changed() {
								s.sync.IntervalMinutes = int(value)
							}
						})
					}
					ui.Field(c, "每日最多次数", func() {
						value := float64(s.sync.DailyLimit)
						if ui.NumberInput(c, &value, 0, 1440, 1).Label("每日最多次数").Width(160).Changed() {
							s.sync.DailyLimit = int(value)
						}
					}).Description("0 表示不限；自动检查和重试均计一次，手动同步不受限。")
					ui.Field(c, "失败重试次数", func() {
						value := float64(s.sync.RetryLimit)
						if ui.NumberInput(c, &value, 0, 10, 1).Label("失败重试次数").Width(160).Changed() {
							s.sync.RetryLimit = int(value)
						}
					})
					ui.Field(c, "每台设备保留备份", func() {
						value := float64(s.sync.KeepSnapshots)
						if ui.NumberInput(c, &value, 1, 100, 1).Label("保留备份数").Width(160).Changed() {
							s.sync.KeepSnapshots = int(value)
						}
					})
				})
				checkbox(c, &s.sync.SyncOnStart, "启动 FreeType 后同步一次")
				ui.Row(c).Gap(10).Children(func() {
					if ui.Button(c, "保存同步计划").Disabled(busy).Clicked() {
						s.syncMessageArea = "plan"
						if s.saveSync() {
							s.syncMessage = "同步计划已保存"
							s.syncFailed = false
						}
					}
					if ui.PrimaryButton(c, "立即同步").Disabled(busy || s.autoSync == nil).Clicked() {
						s.syncMessageArea = "plan"
						if s.saveSync() {
							go s.autoSync.run(time.Now(), true)
						}
					}
					if ui.Button(c, "打开恢复前备份").Clicked() && s.openFolder != nil {
						s.openFolder(filepath.Join(s.syncDataDir(), "sync-backups"))
					}
				})
				s.syncFeedback(c, "plan")
			})
		})
		s.settingBlock(c, "sync/上传与恢复", func() {
			ui.Row(c).Padding(13, 18).Gap(10).AlignItems(ui.Center).Children(func() {
				texts(c, "上传与恢复", "上传会把当前的全部设置和数据存成这台电脑的一份快照；每台电脑保留最近 "+fmt.Sprint(s.sync.KeepSnapshots)+" 份")
				if ui.Button(c, "刷新设备").Disabled(busy).Clicked() {
					s.refreshSyncDevices()
				}
				if ui.PrimaryButton(c, "立即上传").Disabled(busy).Clicked() {
					s.uploadSync()
				}
			})
			ui.Column(c).Padding(0, 18).Children(func() { s.syncFeedback(c, "transfer") })
			if s.syncListed && len(s.syncDevices) == 0 {
				ui.Text(c, "远端还没有快照").TextColor(colors(c).muted).Padding(4, 18, 14, 18)
			}
			for _, device := range s.syncDevices {
				device := device
				name := deviceName(device)
				if device.ID == s.sync.DeviceID {
					name += "（这台电脑）"
				}
				detail := "最近上传：" + time.Unix(device.Uploaded, 0).Format("2006-01-02 15:04") + " · " + sizeText(device.Size)
				ui.Row(c).Key("device-"+device.ID).Padding(11, 18).Gap(12).AlignItems(ui.Center).
					BorderWidth(1, 0, 0, 0).BorderColor(colors(c).border).Children(func() {
					ui.Column(c).Grow(1).Shrink(1).Children(func() {
						ui.Text(c, name).SingleLine()
						ui.Text(c, detail).FontSize(12.5).TextColor(colors(c).muted)
					})
					if ui.Button(c, "恢复…").Label("恢复 " + name).Disabled(busy).Clicked() {
						s.syncRestore, s.confirmSyncRestore = device, true
					}
				})
			}
		})
	})
	if ui.Button(c, "同步说明").Clicked() {
		s.showSyncHelp = !s.showSyncHelp
	}
	if s.showSyncHelp {
		hint(c, "数据只发往你填写的地址，不经过其他服务器。密码和密钥用 Windows 的数据保护加密后保存在本机，不会上传。\n"+
			"设备列表中的手动恢复是合并：把快照里的词库、快捷短语、常用语、学习记录并进本机，并采纳那台电脑上改过的设置；本机已有的内容不会被删除。"+
			"因此在一台电脑上删掉的内容，合并另一台的快照后可能会回来。中英文状态、自定义主词库和悬浮状态栏位置各台电脑各自保留。\n"+
			"自动同步按数据修改时间选择整份备份，较新版本会替换本机全部设置和数据（包括删除）；各台电脑请开启系统自动校时。恢复前的本机备份保存在 sync-backups。\n"+
			"关闭设置窗口后自动同步继续运行；退出 FreeType 设置和托盘后停止。相同内容不会重复传输。")
	}
	title := "合并 " + deviceName(s.syncRestore) + " 的快照？"
	switch ui.AlertDialog(c, &s.confirmSyncRestore, title, "将把这份快照并进本机当前的数据，本机已有的内容不会被删除。", "取消", "合并") {
	case 1:
		s.restoreSync(s.syncRestore)
	}
}

// 操作反馈跟随对应按钮，避免连接结果被页面底部的说明淹没。
func (s *settings) syncFeedback(c *ui.Context, area string) {
	current := s.syncMessageArea
	if current == "" {
		current = "transfer"
	}
	if current != area {
		return
	}
	message, color := s.syncMessage, colors(c).muted
	if s.syncBusy != "" {
		message = s.syncBusy
	} else if s.syncFailed {
		color = c.Theme().Danger
	}
	if message != "" {
		ui.Text(c, message).Key("sync-message-" + area).TextColor(color).Wrap()
	}
}

// 结果、最近时间、下次时间和今日次数各占独立位置，不拼成一段说明。
func (s *settings) syncStatusSummary(c *ui.Context) {
	status := s.autoSyncStatus
	message := status.Message
	if message == "" {
		message = "尚未同步"
	}
	color := c.Theme().Text
	if status.Failed {
		color = c.Theme().Danger
	}
	ui.Column(c).Gap(16).Padding(2, 0, 18, 0).BorderWidth(0, 0, 1, 0).BorderColor(colors(c).border).Children(func() {
		ui.Text(c, message).FontSize(16).TextColor(color).Wrap()
		ui.Row(c).Gap(18).AlignItems(ui.Start).Children(func() {
			for _, item := range []struct {
				label string
				at    time.Time
				empty string
			}{
				{"最近同步", status.Last, "尚未同步"}, {"下次同步", status.Next, "等待安排"},
			} {
				ui.Column(c).Grow(1).Basis(0).Gap(5).Children(func() {
					ui.Text(c, item.label).TextColor(colors(c).muted)
					if item.label == "下次同步" && !s.sync.Automatic {
						ui.Text(c, "未启用").FontSize(20)
					} else if item.at.IsZero() {
						ui.Text(c, item.empty).FontSize(20)
					} else {
						ui.Text(c, item.at.Local().Format("15:04:05")).FontSize(22)
						ui.Text(c, item.at.Local().Format("2006-01-02")).TextColor(colors(c).muted)
					}
				})
			}
			ui.Column(c).Grow(1).Basis(0).Gap(5).Children(func() {
				ui.Text(c, "今日自动尝试").TextColor(colors(c).muted)
				ui.Text(c, fmt.Sprintf("%d 次", status.Count)).FontSize(22)
			})
		})
	})
}
