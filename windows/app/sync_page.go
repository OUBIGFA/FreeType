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
		if !s.sync.Enabled && s.syncMessageArea != "connection" && !s.showSetting("sync/同步服务") && !s.showSetting("sync/上传与恢复") && !s.showSetting("sync/自动同步") && !s.showSetting("sync/同步计划") && !s.showSetting("sync/备份加密") && !s.showSetting("sync/备份密码") {
			return
		}
		s.settingBlock(c, "sync/同步服务", func() {
			ui.Column(c).FillWidth().Padding(2, 12, 8, 12).Children(func() {
				ui.Column(c).FillWidth().Padding(16, rowInset-12).Radius(controlRadius).Background(colors(c).subtle).Gap(16).Children(func() {
					provider := 0
					if s.sync.Provider == "s3" {
						provider = 1
					}
					if ui.Segmented(c, &provider, "WebDAV", "S3 兼容").AlignSelf(ui.Start).Label("同步服务").Changed() {
						s.sync.Provider = []string{"webdav", "s3"}[provider]
					}
					ui.Column(c).FillWidth().Gap(14).Children(func() {
						if s.sync.Provider == "s3" {
							ui.Field(c, "服务地址", func() {
								ui.TextInput(c, &s.sync.S3Endpoint).Placeholder("https://s3.amazonaws.com").FillWidth()
							}).Description("填写 S3 兼容服务地址，如 AWS S3、MinIO、R2 或 OSS")
							ui.Row(c).Gap(12).Wrap().AlignItems(ui.Start).Children(func() {
								ui.Field(c, "区域", func() { ui.TextInput(c, &s.sync.S3Region).Placeholder("us-east-1").FillWidth() }).
									Grow(1).Basis(200).MinWidth(160)
								ui.Field(c, "存储桶", func() { ui.TextInput(c, &s.sync.S3Bucket).FillWidth() }).
									Grow(1).Basis(200).MinWidth(160)
							})
							ui.Field(c, "目录", func() { ui.TextInput(c, &s.sync.S3Prefix).Placeholder("FreeType").FillWidth() }).
								Description("统一保存在此目录下的 FreeType 文件夹中；留空则使用 FreeType")
							ui.Row(c).Gap(12).Wrap().AlignItems(ui.Start).Children(func() {
								ui.Field(c, "Access Key", func() { ui.TextInput(c, &s.sync.S3AccessKey).FillWidth() }).
									Grow(1).Basis(220).MinWidth(180)
								ui.Field(c, "Secret Key", func() { ui.TextInput(c, &s.sync.S3SecretKey).Password().FillWidth() }).
									Grow(1).Basis(220).MinWidth(180)
							})
						} else {
							ui.Field(c, "地址", func() {
								ui.TextInput(c, &s.sync.WebDAVURL).Placeholder("https://dav.jianguoyun.com/dav/FreeType").FillWidth()
							}).Description("数据保存在地址下的 FreeType 文件夹；地址已以 FreeType 结尾时直接使用；坚果云使用应用密码")
							ui.Row(c).Gap(12).Wrap().AlignItems(ui.Start).Children(func() {
								ui.Field(c, "账号", func() { ui.TextInput(c, &s.sync.WebDAVUser).FillWidth() }).
									Grow(1).Basis(220).MinWidth(180)
								ui.Field(c, "密码", func() { ui.TextInput(c, &s.sync.WebDAVPassword).Password().FillWidth() }).
									Grow(1).Basis(220).MinWidth(180)
							})
						}
					})
					if s.sync.Provider == "s3" {
						checkbox(c, &s.sync.S3PathStyle, "路径风格地址；MinIO 等自建服务需勾选，AWS 和 R2 可不勾选")
					}
					ui.Row(c).Gap(12).Wrap().AlignItems(ui.Center).Children(func() {
						if ui.PrimaryButton(c, "保存并测试连接").Disabled(busy).Clicked() {
							s.testSync()
						}
						ui.Column(c).Grow(1).Basis(200).MinWidth(160).Shrink(1).Children(func() { s.syncFeedback(c, "connection") })
					})
				})
			})
		})
		if !s.sync.Enabled && !s.showSetting("sync/上传与恢复") && !s.showSetting("sync/自动同步") && !s.showSetting("sync/同步计划") &&
			!s.showSetting("sync/备份加密") && !s.showSetting("sync/备份密码") {
			return
		}
		s.encryptionRows(c, busy)
	})
	if !s.sync.Enabled && !s.showSetting("sync/上传与恢复") && !s.showSetting("sync/自动同步") && !s.showSetting("sync/同步计划") {
		return
	}
	section(c, "同步计划", func() {
		if s.settingSwitch(c, "sync/自动同步", false, &s.sync.Automatic) {
			s.saveSync()
		}
		s.settingBlock(c, "sync/同步计划", func() {
			ui.Column(c).Padding(8, 0, 14, 0).Gap(16).Children(func() {
				s.syncStatusSummary(c)
				ui.Row(c).Gap(16).Wrap().AlignItems(ui.Start).Children(func() {
					ui.Field(c, "同步方式", func() {
						mode := 0
						if s.sync.Schedule == "daily" {
							mode = 1
						}
						// 表单内容列默认拉伸子控件；分段按钮按标签宽度显示。
						if ui.Segmented(c, &mode, "按间隔", "每天定时").AlignSelf(ui.Start).Label("同步方式").Changed() {
							s.sync.Schedule = []string{"interval", "daily"}[mode]
						}
					}).Grow(1).Basis(200).MinWidth(160)
					if s.sync.Schedule == "daily" {
						ui.Field(c, "每天时间", func() { ui.TextInput(c, &s.sync.DailyTime).Label("每天同步时间").Placeholder("21:00").Width(160) }).
							Description("按本机时间，24 小时制").Grow(1).Basis(200).MinWidth(160)
					} else {
						ui.Field(c, "间隔 / 分钟", func() {
							value := float64(s.sync.IntervalMinutes)
							if ui.NumberInput(c, &value, 1, 1440, 1).Label("同步间隔").Width(160).Changed() {
								s.sync.IntervalMinutes = int(value)
							}
						}).Grow(1).Basis(200).MinWidth(160)
					}
				})
				ui.Row(c).Gap(16).Wrap().AlignItems(ui.Start).Children(func() {
					ui.Field(c, "每日最多次数", func() {
						value := float64(s.sync.DailyLimit)
						if ui.NumberInput(c, &value, 0, 1440, 1).Label("每日最多次数").Width(160).Changed() {
							s.sync.DailyLimit = int(value)
						}
					}).Description("0 表示不限；手动同步不受限").Grow(1).Basis(200).MinWidth(160)
					ui.Field(c, "失败重试次数", func() {
						value := float64(s.sync.RetryLimit)
						if ui.NumberInput(c, &value, 0, 10, 1).Label("失败重试次数").Width(160).Changed() {
							s.sync.RetryLimit = int(value)
						}
					}).Grow(1).Basis(180).MinWidth(160)
					ui.Field(c, "每台电脑保留备份", func() {
						value := float64(s.sync.KeepSnapshots)
						if ui.NumberInput(c, &value, 1, 100, 1).Label("保留备份数").Width(160).Changed() {
							s.sync.KeepSnapshots = int(value)
						}
					}).Grow(1).Basis(180).MinWidth(160)
				})
				checkbox(c, &s.sync.SyncOnStart, "启动 FreeType 后同步一次")
				ui.Row(c).Gap(8).Wrap().Children(func() {
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
	})
	section(c, "电脑备份", func() {
		s.settingBlock(c, "sync/上传与恢复", func() {
			row(c, "上传与恢复", "上传当前全部设置和数据；每台电脑保留最近 "+fmt.Sprint(s.sync.KeepSnapshots)+" 份备份", func() {
				if ui.Button(c, "刷新设备").Disabled(busy).Clicked() {
					s.refreshSyncDevices()
				}
				if ui.PrimaryButton(c, "立即上传").Disabled(busy).Clicked() {
					s.uploadSync()
				}
			})
			ui.Column(c).Padding(0, 0).Children(func() { s.syncFeedback(c, "transfer") })
			if s.syncListed && len(s.syncDevices) == 0 {
				ui.Text(c, "云端还没有备份").TextColor(colors(c).muted).Wrap().Padding(6, 0, 12, 0)
			}
			for _, device := range s.syncDevices {
				device := device
				name := deviceName(device)
				if device.ID == s.sync.DeviceID {
					name += " · 这台电脑"
				}
				detail := "最近上传：" + time.Unix(device.Uploaded, 0).Format("2006-01-02 15:04") + " · " + sizeText(device.Size)
				ui.Column(c).Key("device-"+device.ID).FillWidth().Padding(4, 12).Children(func() {
					ui.Row(c).FillWidth().Padding(12, rowInset-12).Radius(controlRadius).
						Background(colors(c).subtle).Gap(16).Wrap().MinHeight(58).AlignItems(ui.Center).Children(func() {
						texts(c, name, detail)
						if ui.Button(c, "恢复").Label("恢复 " + name).Disabled(busy).Clicked() {
							s.syncRestore, s.confirmSyncRestore = device, true
						}
					})
				})
			}
		})
	})
	ui.Row(c).Margin(8, 0, 0, 0).Children(func() {
		if ui.Button(c, "同步说明").Clicked() {
			s.showSyncHelp = !s.showSyncHelp
		}
	})
	if s.showSyncHelp {
		hint(c, "同步数据只发送到所填地址；账号密码和密钥加密保存在这台电脑，不会上传\n"+
			"备份加密默认关闭；开启后用备份密码加密数据；电脑名、上传时间和大小不加密\n"+
			"所有电脑使用同一备份密码；备份密码不会上传；旧备份仍需旧密码；忘记密码后无法解密\n"+
			"手动恢复会合并词库、快捷短语、常用语和学习记录，并采用备份中的设置；保留已有内容；已删除内容可能重新出现\n"+
			"自动同步按数据修改时间，用较新备份替换全部设置和数据，包括删除记录；请开启系统自动校时；恢复前的备份保存在 sync-backups\n"+
			"每台电脑保留自己的中英文状态、主词库路径和状态栏位置\n"+
			"关闭设置窗口后继续同步；退出设置和托盘后停止同步；相同内容不重复传输")
	}
	title := "合并 " + deviceName(s.syncRestore) + " 的备份"
	switch ui.AlertDialog(c, &s.confirmSyncRestore, title, "合并备份中的数据；保留这台电脑已有内容", "取消", "合并") {
	case 1:
		s.restoreSync(s.syncRestore)
	}
}

// encryptionRows 是云端备份的加密：一个默认关闭的开关，打开后填密码。
// 没开加密时也留着密码一行（定位到它时显示）：别的电脑加密上传的备份要用它打开。
func (s *settings) encryptionRows(c *ui.Context, busy bool) {
	if s.settingSwitch(c, "sync/备份加密", false, &s.sync.Encrypt) {
		s.saveSync()
	}
	if !s.sync.Encrypt && s.sync.EncryptPassword == "" && !s.showSetting("sync/备份密码") {
		return
	}
	s.settingSubRow(c, "sync/备份密码", func() {
		ui.TextInput(c, &s.sync.EncryptPassword).Password().Label("备份密码").Width(220).MaxWidthPercent(100).Shrink(1)
		// 点了才保存：自动同步不会拿打到一半的密码去加密
		if ui.Button(c, "保存密码").Disabled(busy).Clicked() {
			s.syncMessageArea = "encrypt"
			if s.saveSync() {
				s.syncMessage, s.syncFailed = "备份密码已保存；其他电脑要填同一个密码", false
				if s.sync.EncryptPassword == "" {
					s.syncMessage = "备份密码已清空"
				}
			}
		}
	})
	if s.syncMessageArea == "encrypt" && (s.syncBusy != "" || s.syncMessage != "") {
		ui.Column(c).FillWidth().Padding(0, 12, 4, 12).Children(func() {
			ui.Column(c).FillWidth().Padding(0, rowInset-12, 8, subInset-12).Radius(controlRadius).Background(colors(c).subtle).Children(func() {
				s.syncFeedback(c, "encrypt")
			})
		})
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
		ui.Text(c, message).Key("sync-message-" + area).TextColor(color).LineHeight(1.4).Wrap()
	}
}

// 结果、最近时间、下次时间和今日次数各占独立软卡片，不生硬划线分割。
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
	p := colors(c)
	ui.Column(c).Gap(12).Padding(14, 16).Radius(controlRadius).Background(p.subtle).Children(func() {
		ui.Text(c, message).FontSize(textSize(c, 15)).FontWeight(600).TextColor(color).LineHeight(1.4).Wrap()
		ui.Row(c).Gap(16).Wrap().AlignItems(ui.Start).Children(func() {
			for _, item := range []struct {
				label string
				at    time.Time
				empty string
			}{
				{"最近同步", status.Last, "尚未同步"}, {"下次同步", status.Next, "等待安排"},
			} {
				ui.Column(c).Grow(1).Basis(textSize(c, 132)).MinWidth(textSize(c, 118)).Gap(3).Children(func() {
					ui.Text(c, item.label).FontSize(textSize(c, 12)).TextColor(p.muted).Wrap()
					if item.label == "下次同步" && !s.sync.Automatic {
						ui.Text(c, "未启用").FontSize(textSize(c, 18)).FontWeight(600).Wrap()
					} else if item.at.IsZero() {
						ui.Text(c, item.empty).FontSize(textSize(c, 18)).FontWeight(600).Wrap()
					} else {
						ui.Text(c, item.at.Local().Format("15:04:05")).FontSize(textSize(c, 20)).FontWeight(600).FontFeatures("tnum")
						ui.Text(c, item.at.Local().Format("2006-01-02")).FontSize(textSize(c, 12)).TextColor(p.muted)
					}
				})
			}
			ui.Column(c).Grow(1).Basis(textSize(c, 132)).MinWidth(textSize(c, 118)).Gap(3).Children(func() {
				ui.Text(c, "今日自动尝试").FontSize(textSize(c, 12)).TextColor(p.muted).Wrap()
				ui.Text(c, fmt.Sprintf("%d 次", status.Count)).FontSize(textSize(c, 20)).FontWeight(600).FontFeatures("tnum")
			})
		})
	})
}
