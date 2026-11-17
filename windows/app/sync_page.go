package main

import (
	"fmt"
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
	busy := s.syncBusy != ""
	section(c, "云端同步", func() {
		if s.settingSwitch(c, "sync/启用同步", false, &s.sync.Enabled) {
			s.saveSync()
		}
		if !s.sync.Enabled && !s.showSetting("sync/同步服务") && !s.showSetting("sync/上传与恢复") {
			return
		}
		s.settingBlock(c, "sync/同步服务", func() {
			ui.Column(c).Padding(16, 18).Gap(12).Children(func() {
				provider := 0
				if s.sync.Provider == "s3" {
					provider = 1
				}
				if ui.Segmented(c, &provider, "WebDAV", "S3 兼容").Label("同步服务").Changed() {
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
								Description("快照放在存储桶里的哪个目录下，可以留空。")
							ui.Field(c, "Access Key", func() { ui.TextInput(c, &s.sync.S3AccessKey).Width(360) })
							ui.Field(c, "Secret Key", func() { ui.TextInput(c, &s.sync.S3SecretKey).Password().Width(360) })
						} else {
							ui.Field(c, "地址", func() {
								ui.TextInput(c, &s.sync.WebDAVURL).Placeholder("https://dav.jianguoyun.com/dav/FreeType").Width(360)
							}).Description("WebDAV 文件夹的完整地址。坚果云需在网页端的「安全选项」里生成应用密码。")
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
				})
			})
		})
		s.settingBlock(c, "sync/上传与恢复", func() {
			ui.Row(c).Padding(13, 18).Gap(10).AlignItems(ui.Center).Children(func() {
				texts(c, "上传与恢复", "上传会把当前的全部设置和数据存成这台电脑的一份快照；每台电脑保留最近 "+fmt.Sprint(keepSnapshots)+" 份")
				if ui.Button(c, "刷新设备").Disabled(busy).Clicked() {
					s.refreshSyncDevices()
				}
				if ui.PrimaryButton(c, "立即上传").Disabled(busy).Clicked() {
					s.uploadSync()
				}
			})
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
	if busy || s.syncMessage != "" {
		message, color := s.syncMessage, colors(c).muted
		if busy {
			message = s.syncBusy
		} else if s.syncFailed {
			color = c.Theme().Danger
		}
		ui.Text(c, message).Key("sync-message").TextColor(color).Wrap().Margin(10, 2, 0, 2)
	}
	hint(c, "数据只发往你填写的地址，不经过其他服务器。密码和密钥用 Windows 的数据保护加密后保存在本机，不会上传。\n"+
		"恢复是合并：把快照里的词库、快捷短语、常用语、学习记录并进本机，并采纳那台电脑上改过的设置；本机已有的内容不会被删除。"+
		"因此在一台电脑上删掉的内容，合并另一台的快照后可能会回来。中英文状态、自定义主词库和悬浮状态栏位置各台电脑各自保留。\n"+
		"目前需要手动点「立即上传」，没有自动上传。")
	title := "合并 " + deviceName(s.syncRestore) + " 的快照？"
	switch ui.AlertDialog(c, &s.confirmSyncRestore, title, "将把这份快照并进本机当前的数据，本机已有的内容不会被删除。", "取消", "合并") {
	case 1:
		s.restoreSync(s.syncRestore)
	}
}
