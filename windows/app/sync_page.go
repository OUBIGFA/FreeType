package main

import (
	"fmt"
	"path/filepath"
	"time"

	"github.com/egoist/mygo/ui"
)

// syncPage 是「数据与同步」页：整份数据的导入导出，以及上传到自己的网盘、在别的电脑上合并恢复。
func (s *settings) syncPage(c *ui.Context) {
	pageHeader(c, "数据与同步", "")
	s.backupSection(c)
	if !s.loaded {
		return
	}
	s.loadSync()
	busy := s.syncBusy != "" || s.autoSyncStatus.Busy
	section(c, "云端同步", func() {
		showConnection := s.sync.Enabled || s.syncMessageArea == "connection" || s.showSetting("sync/同步服务") ||
			s.showSetting("sync/上传与恢复") || s.showSetting("sync/自动同步") || s.showSetting("sync/同步计划") ||
			s.showSetting("sync/备份加密") || s.showSetting("sync/备份密码")
		settingGroup(c, func() {
			if s.settingSwitch(c, "sync/启用同步", false, &s.sync.Enabled) {
				s.saveSync()
			}
		}, showConnection, func() {
			s.settingBlock(c, "sync/同步服务", func() {
				subContent(c, func() {
					provider := 0
					if s.sync.Provider == "s3" {
						provider = 1
					}
					if ui.Segmented(c, &provider, "WebDAV", "S3 兼容").AlignSelf(ui.Start).Label("同步服务").Changed() {
						s.sync.Provider = []string{"webdav", "s3"}[provider]
					}
					ui.Column(c).FillWidth().Gap(16).Children(func() {
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
	if !s.sync.Enabled && s.syncPreview == nil && !s.showSetting("sync/上传与恢复") && !s.showSetting("sync/自动同步") && !s.showSetting("sync/同步计划") {
		return
	}
	section(c, "同步计划", func() {
		settingGroup(c, func() {
			if s.settingSwitch(c, "sync/自动同步", false, &s.sync.Automatic) {
				s.saveSync()
			}
		}, true, func() {
			s.settingBlock(c, "sync/同步计划", func() {
				showPlan := s.sync.Automatic || s.showSetting("sync/同步计划")
				ui.Column(c).FillWidth().Gap(4).Dividers(1, colors(c).border).Children(func() {
					s.syncStatusSummary(c)
					if showPlan {
						ui.Column(c).FillWidth().Children(func() {
							subRow(c, "同步方式", "", func() {
								mode := 0
								if s.sync.Schedule == "daily" {
									mode = 1
								}
								if ui.Segmented(c, &mode, "按间隔", "每天定时").Label("同步方式").Changed() {
									s.sync.Schedule = []string{"interval", "daily"}[mode]
								}
							})
							nestedSubGroup(c, func() {
								if s.sync.Schedule == "daily" {
									subRow(c, "每天时间", "按本机时间，24 小时制", func() {
										ui.TextInput(c, &s.sync.DailyTime).Label("每天同步时间").Placeholder("21:00").Width(160).MaxWidthPercent(100)
									})
								} else {
									subRow(c, "同步间隔 / 分钟", "", func() {
										value := float64(s.sync.IntervalMinutes)
										if ui.NumberInput(c, &value, 1, 1440, 1).Label("同步间隔").Width(120).Changed() {
											s.sync.IntervalMinutes = int(value)
										}
									})
								}
							})
						})
						switchLine(c, "启动时同步", "启动 FreeType 后同步一次", &s.sync.SyncOnStart, true)
						subRow(c, "每日最多次数", "0 表示不限；手动同步不受限", func() {
							value := float64(s.sync.DailyLimit)
							if ui.NumberInput(c, &value, 0, 1440, 1).Label("每日最多次数").Width(120).Changed() {
								s.sync.DailyLimit = int(value)
							}
						})
						subRow(c, "失败重试次数", "0 表示不重试", func() {
							value := float64(s.sync.RetryLimit)
							if ui.NumberInput(c, &value, 0, 10, 1).Label("失败重试次数").Width(120).Changed() {
								s.sync.RetryLimit = int(value)
							}
						})
						subRow(c, "每台电脑保留备份", "超出时自动清理较早的云端备份", func() {
							value := float64(s.sync.KeepSnapshots)
							if ui.NumberInput(c, &value, 1, 100, 1).Label("保留备份数").Width(120).Changed() {
								s.sync.KeepSnapshots = int(value)
							}
						})
					}
					ui.Column(c).FillWidth().Padding(12, 0, 8, 0).Gap(8).Children(func() {
						ui.Row(c).Gap(8).Wrap().AlignItems(ui.Center).Children(func() {
							if showPlan && ui.Button(c, "保存同步计划").Disabled(busy).Clicked() {
								s.syncMessageArea = "plan"
								if s.saveSync() {
									s.syncMessage = "同步计划已保存"
									s.syncFailed = false
								}
							}
							if ui.PrimaryButton(c, "立即同步").Disabled(busy || s.autoSync == nil).Clicked() {
								s.retrySync()
							}
							if ui.Button(c, "打开恢复前备份").Clicked() && s.openFolder != nil {
								s.openFolder(filepath.Join(s.syncDataDir(), "sync-backups"))
							}
						})
						s.syncFeedback(c, "plan")
						if !s.autoSyncStatus.Failed && s.syncMessageArea == "plan" && s.syncFailed && s.syncMessage != "" {
							s.syncRecoveryActions(c, "plan", s.syncMessage, busy)
						}
					})
				})
			})
		})
	})
	section(c, "电脑备份", func() {
		s.settingBlock(c, "sync/上传与恢复", func() {
			hasDevices := len(s.syncDevices) > 0 || (s.syncListed && len(s.syncDevices) == 0)
			settingGroup(c, func() {
				row(c, "上传与恢复", "上传当前全部设置和数据；每台电脑保留最近 "+fmt.Sprint(s.sync.KeepSnapshots)+" 份备份", func() {
					if ui.Button(c, "刷新设备").Disabled(busy).Clicked() {
						s.refreshSyncDevices()
					}
					if ui.PrimaryButton(c, "立即上传").Disabled(busy).Clicked() {
						s.uploadSync()
					}
				})
				ui.Column(c).FillWidth().Padding(0, 0).Gap(8).Children(func() {
					s.syncFeedback(c, "transfer")
					if (s.syncMessageArea == "transfer" || s.syncMessageArea == "") && s.syncFailed && s.syncMessage != "" {
						s.syncRecoveryActions(c, "transfer", s.syncMessage, busy)
					}
				})
			}, hasDevices, func() {
				if s.syncListed && len(s.syncDevices) == 0 {
					ui.Text(c, "云端还没有备份").FontSize(textSize(c, fontCaption)).TextColor(colors(c).muted).Wrap().Padding(8, 0)
					return
				}
				for _, device := range s.syncDevices {
					device := device
					name := deviceName(device)
					if device.ID == s.sync.DeviceID {
						name += " · 这台电脑"
					}
					detail := "最近上传：" + time.Unix(device.Uploaded, 0).Format("2006-01-02 15:04") + " · " + sizeText(device.Size)
					ui.Column(c).Key("device-" + device.ID).FillWidth().Children(func() {
						subRow(c, name, detail, func() {
							if ui.Button(c, "恢复").Label("恢复 " + name).Disabled(busy).Clicked() {
								s.previewSync(device)
							}
						})
					})
				}
			})
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
			"恢复前显示新增、更新、删除和冲突；合并按每条记录的时间处理，两台电脑的独立新增都会保留\n"+
			"删除标记会随同步保存，旧快照不会复活已删除内容；旧格式没有删除记录，只补缺少内容；请开启系统自动校时\n"+
			"自动同步合并各台设备的最新快照；整份替换可在恢复预览中主动选择；恢复前备份保存在 sync-backups\n"+
			"每台电脑保留自己的中英文状态、主词库路径和状态栏位置\n"+
			"关闭设置窗口后继续同步；退出设置和托盘后停止同步；相同内容不重复传输")
	}
	if preview := s.syncPreview; preview != nil {
		section(c, "恢复预览", func() {
			hint(c, fmt.Sprintf("新增 %d 项；更新 %d 项；删除 %d 项；冲突 %d 项", preview.Added, preview.Updated, preview.Deleted, preview.Conflicts))
			hint(c, "冲突按较新的记录处理；同一时间删除优先；下方最多显示 100 条影响项")
			for _, detail := range preview.Details {
				ui.Text(c, detail).Wrap().FontSize(textSize(c, fontCaption))
			}
			checkbox(c, &s.syncReplace, "整份替换本机数据").Disabled(busy)
			if s.syncReplace {
				hint(c, "将恢复备份中的全部内容，包括旧词条和设置；本机之后新增的内容会被替换；恢复前留存备份")
			}
			ui.Row(c).Gap(12).Children(func() {
				if ui.Button(c, "取消预览").Disabled(busy).Clicked() {
					s.cancelSyncPreview()
				}
				label := "确认合并"
				if s.syncReplace {
					label = "确认整份替换"
				}
				if ui.PrimaryButton(c, label).Disabled(busy).Clicked() {
					s.restoreSync(s.syncRestore)
				}
			})
		})
	}
}

// encryptionRows 是云端备份的加密：一个默认关闭的开关，打开后填密码。
// 没开加密时也留着密码一行（定位到它时显示）：别的电脑加密上传的备份要用它打开。
func (s *settings) encryptionRows(c *ui.Context, busy bool) {
	showPassword := s.sync.Encrypt || s.sync.EncryptPassword != "" || s.showSetting("sync/备份密码")
	settingGroup(c, func() {
		if s.settingSwitch(c, "sync/备份加密", false, &s.sync.Encrypt) {
			s.saveSync()
		}
	}, showPassword, func() {
		ui.Column(c).FillWidth().Children(func() {
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
				ui.Column(c).FillWidth().Padding(0, 0, 8, 0).Children(func() {
					s.syncFeedback(c, "encrypt")
				})
			}
		})
	})
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

// 在次级容器内用底部分隔线区隔同步状态摘要与下方表单，不嵌套第二层卡片。
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
	ui.Column(c).FillWidth().Gap(12).Padding(12, 0, 12, 0).Children(func() {
		ui.Text(c, message).FontSize(textSize(c, fontBody)).FontWeight(600).TextColor(color).LineHeight(1.4).Wrap()
		if status.Failed {
			s.syncRecoveryActions(c, "plan", message, s.syncBusy != "" || status.Busy)
		}
		ui.Row(c).Gap(16).Wrap().AlignItems(ui.Start).Children(func() {
			for _, item := range []struct {
				label string
				at    time.Time
				empty string
			}{
				{"最近同步", status.Last, "尚未同步"}, {"下次同步", status.Next, "等待安排"},
			} {
				ui.Column(c).Grow(1).Basis(textSize(c, 132)).MinWidth(textSize(c, 118)).Gap(4).Children(func() {
					ui.Text(c, item.label).FontSize(textSize(c, fontCaption)).TextColor(p.muted).Wrap()
					if item.label == "下次同步" && !s.sync.Automatic {
						ui.Text(c, "未启用").FontSize(textSize(c, fontHeading)).FontWeight(600).Wrap()
					} else if item.at.IsZero() {
						ui.Text(c, item.empty).FontSize(textSize(c, fontHeading)).FontWeight(600).Wrap()
					} else {
						ui.Text(c, item.at.Local().Format("15:04:05")).FontSize(textSize(c, fontHeading)).FontWeight(600).FontFeatures("tnum")
						ui.Text(c, item.at.Local().Format("2006-01-02")).FontSize(textSize(c, fontCaption)).TextColor(p.muted).FontFeatures("tnum")
					}
				})
			}
			ui.Column(c).Grow(1).Basis(textSize(c, 132)).MinWidth(textSize(c, 118)).Gap(4).Children(func() {
				ui.Text(c, "今日自动尝试").FontSize(textSize(c, fontCaption)).TextColor(p.muted).Wrap()
				ui.Text(c, fmt.Sprintf("%d 次", status.Count)).FontSize(textSize(c, fontHeading)).FontWeight(600).FontFeatures("tnum")
			})
		})
	})
}

func (s *settings) syncRecoveryActions(c *ui.Context, area, message string, busy bool) {
	p := colors(c)
	kind := classifySyncProblem(message)
	var guidance string
	switch kind {
	case syncProblemRecordsMismatch:
		guidance = "同步记录与当前数据不一致，可能是跨版本同步或记录文件损坏；可先重建本机记录后重试，仍失败时可按本机或云端备份重置"
	case syncProblemCorruptRemote:
		guidance = "云端备份文件可能未传完、被修改或已损坏；可刷新设备列表确认，或用本机数据重新初始化云端"
	case syncProblemPassword:
		guidance = "云端存在加密备份或备份密码不匹配；请先核对并保存备份密码后再重试，若已遗忘密码可用本机数据重置云端"
	case syncProblemConnection:
		guidance = "无法连接同步服务或凭据权限不足；请检查地址、账号密码与网络后再重试"
	case syncProblemTransient:
		guidance = "同步期间本机或云端数据发生变化，或正在输入中；可直接重试一次"
	default:
		guidance = "同步未能完成；可先重试一次，仍失败时可重建本机同步记录或以本机数据重置云端"
	}
	// 故障说明只用一块浅底色标出，确认步骤用细线隔开，不再在里面套框。
	ui.Column(c).Key("sync-recovery-"+area).FillWidth().Padding(space3, space4).Radius(controlRadius).
		Background(p.subtle).Gap(space3).Children(func() {
		ui.Text(c, guidance).FontSize(textSize(c, fontCaption)).TextColor(p.muted).LineHeight(1.45).Wrap()
		ui.Row(c).FillWidth().Wrap().GapX(8).GapY(8).AlignItems(ui.Center).Children(func() {
			switch kind {
			case syncProblemRecordsMismatch:
				if ui.Button(c, "重建本机同步记录并重试").Disabled(busy).Clicked() {
					s.repairLocalSyncRecordsAndRetry()
				}
				if area == "transfer" && s.syncFailedDevice != nil {
					if ui.Button(c, "忽略同步记录并恢复").Disabled(busy).Clicked() {
						s.confirmForceRestore = true
						s.confirmSyncResetCloud = false
					}
				} else if area == "plan" {
					if ui.Button(c, "查看云端备份").Disabled(busy).Clicked() {
						s.locateSetting(setting("sync/上传与恢复"))
						s.refreshSyncDevices()
					}
				}
				if ui.Button(c, "以本机数据重置云端").Disabled(busy).Clicked() {
					s.confirmSyncResetCloud = true
					s.confirmForceRestore = false
				}
			case syncProblemCorruptRemote:
				if ui.Button(c, "刷新设备列表").Disabled(busy).Clicked() {
					s.refreshSyncDevices()
				}
				if ui.Button(c, "以本机数据重置云端").Disabled(busy).Clicked() {
					s.confirmSyncResetCloud = true
					s.confirmForceRestore = false
				}
			case syncProblemPassword:
				if ui.Button(c, "填写备份密码").Disabled(busy).Clicked() {
					s.locateSetting(setting("sync/备份密码"))
				}
				if ui.Button(c, "重试同步").Disabled(busy || s.autoSync == nil).Clicked() {
					s.retrySync()
				}
				if ui.Button(c, "以本机数据重置云端").Disabled(busy).Clicked() {
					s.confirmSyncResetCloud = true
					s.confirmForceRestore = false
				}
			case syncProblemConnection:
				if ui.Button(c, "定位到同步服务").Disabled(busy).Clicked() {
					s.locateSetting(setting("sync/同步服务"))
				}
				if ui.Button(c, "测试连接").Disabled(busy).Clicked() {
					s.testSync()
				}
				if ui.Button(c, "重试同步").Disabled(busy || s.autoSync == nil).Clicked() {
					s.retrySync()
				}
			case syncProblemTransient:
				if area == "transfer" && s.syncFailedDevice != nil {
					if ui.Button(c, "重新预览").Disabled(busy).Clicked() {
						s.previewSync(*s.syncFailedDevice)
					}
				} else if ui.Button(c, "立即重试").Disabled(busy || s.autoSync == nil).Clicked() {
					s.retrySync()
				}
			default:
				if ui.Button(c, "立即重试").Disabled(busy || s.autoSync == nil).Clicked() {
					s.retrySync()
				}
				if ui.Button(c, "重建本机同步记录并重试").Disabled(busy).Clicked() {
					s.repairLocalSyncRecordsAndRetry()
				}
				if ui.Button(c, "以本机数据重置云端").Disabled(busy).Clicked() {
					s.confirmSyncResetCloud = true
					s.confirmForceRestore = false
				}
			}
		})
		if s.confirmSyncResetCloud {
			ui.Column(c).FillWidth().Padding(space3, 0, 0, 0).BorderWidth(1, 0, 0, 0).BorderColor(p.border).Gap(space2).Children(func() {
				ui.Text(c, "将先在本机保存恢复前备份，再删除云端旧同步文件并上传当前数据；其他电脑的旧备份将被清理，请确认本机数据完整后再继续").
					FontSize(textSize(c, fontCaption)).LineHeight(1.45).Wrap()
				ui.Row(c).FillWidth().Wrap().Gap(8).Children(func() {
					if ui.Button(c, "取消").Disabled(busy).Clicked() {
						s.confirmSyncResetCloud = false
					}
					if ui.PrimaryButton(c, "确认以本机重置云端").Disabled(busy).Clicked() {
						s.resetCloudWithLocal()
					}
				})
			})
		}
		if s.confirmForceRestore && s.syncFailedDevice != nil && area == "transfer" {
			ui.Column(c).FillWidth().Padding(space3, 0, 0, 0).BorderWidth(1, 0, 0, 0).BorderColor(p.border).Gap(space2).Children(func() {
				ui.Text(c, "将先保存本机恢复前备份，再忽略该备份的同步记录并按正文整份替换本机数据，同时重建同步记录").
					FontSize(textSize(c, fontCaption)).LineHeight(1.45).Wrap()
				ui.Row(c).FillWidth().Wrap().Gap(8).Children(func() {
					if ui.Button(c, "取消").Disabled(busy).Clicked() {
						s.confirmForceRestore = false
					}
					if ui.PrimaryButton(c, "确认按正文恢复").Disabled(busy).Clicked() {
						s.forceRestoreSync(*s.syncFailedDevice)
					}
				})
			})
		}
	})
}
