package main

import (
	"fmt"
	"strings"

	"github.com/egoist/mygo/ui"
)

var presetModes = []choice{{"english", "英文"}, {"chinese", "中文"}}

// Vim 规则与切入程序时的语言预设独立；不要求用户启用切入自动切换。
func (s *settings) vimSection(c *ui.Context) {
	section(c, "Vim 模式", func() {
		s.settingBlock(c, "features/Vim 程序", func() {
			ui.Row(c).FillWidth().Padding(space3, 0).Gap(space3).Wrap().AlignItems(ui.Center).Children(func() {
				ui.TextInput(c, &s.newVimApp).Label("Vim 程序名").Placeholder("输入程序名，例如 code.exe 或 gvim.exe").
					Grow(1).Basis(240).MinWidth(180).MaxWidthPercent(100).Shrink(1)
				if ui.PrimaryButton(c, "添加程序").Label("添加 Vim 程序").Clicked() {
					if app, valid := normalizePresetApp(s.newVimApp); valid {
						found := false
						for _, existing := range s.config.VimModeApps {
							found = found || existing == app
						}
						if !found {
							s.config.VimModeApps = append(s.config.VimModeApps, app)
							s.saveConfig()
						}
						s.newVimApp = ""
					} else {
						s.notice = "请输入有效的程序名"
					}
				}
			})
		})
		remove := -1
		for i, app := range s.config.VimModeApps {
			row(c, app, "Esc 切英文", func() {
				if ui.Button(c, "移除").Label("移除 Vim 程序 " + app).Clicked() {
					remove = i
				}
			})
		}
		if remove >= 0 {
			s.config.VimModeApps = append(s.config.VimModeApps[:remove], s.config.VimModeApps[remove+1:]...)
			if s.config.VimModeApps == nil {
				s.config.VimModeApps = []string{}
			}
			s.saveConfig()
		}
	})
	hint(c, "取消组字后，再按一次 Esc 切英文；Esc 仍交给程序；同一程序的所有窗口都生效；终端内其他程序也会受到影响")
}

// normalizePresetApp 让设置端和服务端使用同一种规则键：只保留文件名并忽略大小写。
func normalizePresetApp(name string) (string, bool) {
	name = strings.Trim(strings.TrimSpace(name), "\"")
	if i := strings.LastIndexAny(name, `\\/`); i >= 0 {
		name = name[i+1:]
	}
	name = strings.ToLower(strings.TrimSpace(name))
	if name == "" || strings.ContainsAny(name, "<>:\"|?*") || strings.IndexFunc(name, func(r rune) bool { return r < 0x20 }) >= 0 {
		return "", false
	}
	if !strings.HasSuffix(name, ".exe") {
		name += ".exe"
	}
	return name, true
}

func (s *settings) refreshApps() {
	s.background(func() func() {
		response, err := s.backend.Call(op("list_apps"))
		return func() {
			if err != nil {
				s.notice = "读取打开的窗口失败：" + err.Error()
				return
			}
			s.apps = response.Apps
			s.notice = fmt.Sprintf("已找到 %d 个打开的程序", len(s.apps))
		}
	})
}

func (s *settings) savePresetRules(rules []AppPreset) {
	if rules == nil {
		rules = []AppPreset{}
	}
	s.config.AppPresetRules = rules
	s.saveConfig()
}

func (s *settings) addPreset() {
	app, ok := normalizePresetApp(s.newPresetApp)
	if !ok {
		s.presetError = "请输入有效的程序名，例如 code.exe；也可以填写程序的完整路径"
		return
	}
	mode := s.newPresetMode
	if mode != "english" && mode != "chinese" {
		mode = "english"
	}
	rules := append([]AppPreset(nil), s.config.AppPresetRules...)
	for i := range rules {
		if normalized, valid := normalizePresetApp(rules[i].App); valid && normalized == app {
			rules[i] = AppPreset{App: app, Mode: mode}
			s.presetError = ""
			s.savePresetRules(rules)
			return
		}
	}
	rules = append(rules, AppPreset{App: app, Mode: mode})
	s.presetError = ""
	s.savePresetRules(rules)
}

func (s *settings) appPresetSection(c *ui.Context) {
	section(c, "指定软件自动切换", func() {
		showSub := s.config.AppPresets || s.showSetting("features/指定软件规则") || s.showSetting("features/指定软件规则列表")
		settingGroup(c, func() {
			if s.settingSwitch(c, "features/指定软件自动切换", false, &s.config.AppPresets) {
				s.saveConfig()
			}
		}, showSub, func() {
			s.settingBlock(c, "features/指定软件规则", func() {
				subContent(c, func() {
					caption(c, "进入指定程序时应用一次预设；可手动切换；离开后恢复原状态；其他程序和全局状态不受影响；重启后保留规则")
					ui.Row(c).Gap(space3).Wrap().AlignItems(ui.Center).Children(func() {
						if len(s.apps) > 0 {
							options := []string{"手动输入"}
							selected := "手动输入"
							for i, app := range s.apps {
								options = append(options, app.App+" — "+app.Title)
								if normalized, valid := normalizePresetApp(s.newPresetApp); valid && normalized == app.App {
									selected = options[i+1]
								}
							}
							if ui.Select(c, &selected, options).Label("打开的窗口").Width(280).MaxWidthPercent(100).Shrink(1).Changed() && selected != "手动输入" {
								for i, app := range s.apps {
									if options[i+1] == selected {
										s.newPresetApp = app.App
										break
									}
								}
							}
						}
						if ui.Button(c, "刷新打开的窗口").Clicked() {
							s.refreshApps()
						}
					})
					ui.Row(c).Gap(space3).Wrap().AlignItems(ui.Center).Children(func() {
						ui.TextInput(c, &s.newPresetApp).Label("指定程序").Placeholder("程序名或完整路径，例如 code.exe").Grow(1).Basis(240).MinWidth(180).MaxWidthPercent(100).Shrink(1)
						mode := 0
						if s.newPresetMode == "chinese" {
							mode = 1
						}
						if ui.Segmented(c, &mode, "切换为英文", "切换为中文").Label("切入时").Changed() {
							if mode == 0 {
								s.newPresetMode = "english"
							} else {
								s.newPresetMode = "chinese"
							}
						}
						if ui.PrimaryButton(c, "添加 / 更新").Clicked() {
							s.addPreset()
						}
					})
					if s.presetError != "" {
						ui.Text(c, s.presetError).FontSize(textSize(c, fontCaption)).TextColor(c.Theme().Danger).Wrap()
					}
				})
			})
			s.settingBlock(c, "features/指定软件规则列表", func() {
				remove := -1
				change := -1
				changeMode := ""
				if len(s.config.AppPresetRules) == 0 {
					caption(c, "还没有指定软件规则").Padding(space3, 0)
					return
				}
				ui.List(c, nil, len(s.config.AppPresetRules), func(i int) {
					rule := s.config.AppPresetRules[i]
					mode := rule.Mode
					item := ui.Row(c).Padding(space2, 0).MinHeight(subRowMin).Gap(space3).AlignItems(ui.Center)
					item.Children(func() {
						ui.Column(c).Grow(1).Basis(0).MinWidth(0).Shrink(1).Gap(space1).Children(func() {
							ui.Text(c, rule.App).SingleLine()
							caption(c, "切入时："+nameOf(presetModes, mode))
						})
						if choose(c, "模式", presetModes, &mode, 130) {
							change, changeMode = i, mode
						}
						onIntent(c, item, func() {
							if ui.Button(c, "删除").Label("删除 " + rule.App).Clicked() {
								remove = i
							}
						})
					})
				})
				if change >= 0 {
					rules := append([]AppPreset(nil), s.config.AppPresetRules...)
					rules[change].Mode = changeMode
					s.savePresetRules(rules)
				} else if remove >= 0 {
					rules := append([]AppPreset(nil), s.config.AppPresetRules[:remove]...)
					rules = append(rules, s.config.AppPresetRules[remove+1:]...)
					s.savePresetRules(rules)
				}
			})
		})
	})
}
