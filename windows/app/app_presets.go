package main

import (
	"fmt"
	"strings"

	"github.com/egoist/mygo/ui"
)

var presetModes = []choice{{"english", "英文"}, {"chinese", "中文"}}

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
		changed := s.settingSwitch(c, "features/指定软件自动切换", false, &s.config.AppPresets)
		if changed {
			s.saveConfig()
		}
		if !s.config.AppPresets && !s.showSetting("features/指定软件规则") && !s.showSetting("features/指定软件规则列表") {
			return
		}
		s.settingBlock(c, "features/指定软件规则", func() {
			p := colors(c)
			ui.Column(c).FillWidth().Padding(2, 12, 6, 12).Children(func() {
				ui.Column(c).FillWidth().Padding(14, rowInset-12).Radius(controlRadius).Background(p.subtle).Gap(14).Children(func() {
					ui.Text(c, "进入指定程序时应用一次预设；可手动切换；离开后恢复原状态；其他程序和全局状态不受影响；重启后保留规则").
						FontSize(textSize(c, 13)).LineHeight(1.45).TextColor(p.muted).Wrap()
					ui.Row(c).Gap(12).Wrap().AlignItems(ui.Center).Children(func() {
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
					ui.Row(c).Gap(12).Wrap().AlignItems(ui.Center).Children(func() {
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
						ui.Text(c, s.presetError).FontSize(textSize(c, 13)).TextColor(c.Theme().Danger).Wrap()
					}
				})
			})
		})
		s.settingBlock(c, "features/指定软件规则列表", func() {
			remove := -1
			change := -1
			changeMode := ""
			if len(s.config.AppPresetRules) == 0 {
				ui.Text(c, "还没有指定软件规则").FontSize(textSize(c, 13)).TextColor(colors(c).muted).Padding(10, 0, 12, 0)
			}
			ui.List(c, nil, len(s.config.AppPresetRules), func(i int) {
				rule := s.config.AppPresetRules[i]
				mode := rule.Mode
				item := ui.Row(c).Padding(10, 0).MinHeight(58).Gap(12).AlignItems(ui.Center)
				if item.Hovered() {
					item.Background(colors(c).cardHover)
				}
				item.Children(func() {
					ui.Column(c).Grow(1).Basis(0).MinWidth(0).Shrink(1).Gap(3).Children(func() {
						ui.Text(c, rule.App).FontWeight(500).SingleLine()
						ui.Text(c, "切入时："+nameOf(presetModes, mode)).FontSize(textSize(c, 13)).TextColor(colors(c).muted).Wrap()
					})
					if choose(c, "模式", presetModes, &mode, 130) {
						change, changeMode = i, mode
					}
					if ui.Button(c, "删除").Label("删除 " + rule.App).Clicked() {
						remove = i
					}
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
}
