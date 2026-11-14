package main

import (
	"embed"
	"path/filepath"

	"github.com/egoist/mygo/ui"
)

const repositoryURL = "https://github.com/OUBIGFA/FreeType"

// Keep the GitHub mark and its MIT notice together in the distributed app.
//
//go:embed assets/github.svg assets/*.txt
var aboutAssets embed.FS

var githubIcon = func() *ui.SVG {
	data, err := aboutAssets.ReadFile("assets/github.svg")
	if err != nil {
		panic(err)
	}
	return ui.MustParseSVG(data)
}()

func (s *settings) openLink(url string) {
	if s.openURL != nil {
		if err := s.openURL(url); err != nil {
			s.notice = "无法打开链接，请在浏览器中打开：" + url
		}
	}
}

func (s *settings) aboutPage(c *ui.Context) {
	pageHeader(c, "关于", "")
	section(c, "", func() {
		row(c, "FreeType "+s.version, "适用于 Windows 的中文输入法。支持全拼、双拼和中英混输，并根据你的选词习惯调整候选顺序。", nil)
		s.settingBlock(c, "about/GitHub", func() {
			ui.Row(c).Padding(13, 18).Gap(16).MinHeight(60).AlignItems(ui.Center).Children(func() {
				ui.Icon(c, githubIcon).FontSize(20)
				texts(c, "GitHub", repositoryURL)
				if ui.Button(c, "打开仓库").Clicked() {
					s.openLink(repositoryURL)
				}
			})
		})
		s.settingRow(c, "about/问题反馈", func() {
			if ui.Button(c, "反馈问题").Clicked() {
				s.openLink(repositoryURL + "/issues")
			}
		})
	})
	section(c, "软件更新", func() {
		message := s.updateStatus.Message
		if message == "" {
			message = "点击“检查更新”查看可用版本"
		}
		s.settingRowValue(c, "about/检查更新", message, func() {
			if ui.Button(c, "检查更新").Label("检查软件更新").Disabled(s.updateStatus.Busy || s.onCheckUpdates == nil).Clicked() {
				s.onCheckUpdates()
			}
			if ui.Button(c, "立即更新").Disabled(s.updateStatus.Busy || !s.updateStatus.available() || s.onUpdateNow == nil).Clicked() {
				s.onUpdateNow()
			}
		})
		if s.loaded {
			changed := s.settingSwitch(c, "about/自动更新", false, &s.config.AutoUpdate)
			changed = s.settingSwitch(c, "about/更新安装后显示设置页", false, &s.config.ShowSettingsAfterUpdate) || changed
			if changed {
				s.saveConfig()
			}
		} else {
			s.settingRowValue(c, "about/自动更新", "正在读取更新设置…", nil)
			s.settingRowValue(c, "about/更新安装后显示设置页", "正在读取更新设置…", nil)
		}
	})
	section(c, "数据与日志", func() {
		if dir := s.status.DataDir; dir != "" {
			s.settingRowValue(c, "about/用户数据", dir, func() {
				if ui.Button(c, "打开文件夹").Label("打开用户数据文件夹").Clicked() && s.openFolder != nil {
					s.openFolder(dir)
				}
			})
		}
		if dir := s.status.LogDir; dir != "" {
			s.settingRow(c, "about/日志", func() {
				if ui.Button(c, "打开文件夹").Label("打开日志文件夹").Clicked() && s.openFolder != nil {
					s.openFolder(dir)
				}
			})
		}
	})
	section(c, "来源与许可", func() {
		s.settingRow(c, "about/开源信息", func() {
			if ui.Button(c, "查看开源说明").Clicked() {
				s.openLink(repositoryURL + "/tree/main/third_party")
			}
		})
		s.settingRow(c, "about/词库", func() {
			if ui.Button(c, "查看许可文件").Clicked() && s.openFolder != nil {
				s.openFolder(filepath.Join(s.resources, "data"))
			}
		})
	})
}
