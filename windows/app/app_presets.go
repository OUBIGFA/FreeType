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
		s.presetError = "请输入有效的程序名，例如 code.exe。也可以填写程序的完整路径。"
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
		changed := s.settingSwitch(c, "general/指定软件自动切换", false, &s.config.AppPresets)
		if changed {
			s.saveConfig()
		}
		if !s.config.AppPresets && !s.showSetting("general/指定软件规则") && !s.showSetting("general/指定软件规则列表") {
			return
		}
		s.settingBlock(c, "general/指定软件规则", func() {
			p := colors(c)
			ui.Column(c).Padding(14, 18).Gap(10).Children(func() {
				ui.Text(c, "只有从其他程序切入时应用一次；在程序内部手动切换后会保持。没有规则的程序不受影响。\n规则保存在 FreeType 用户设置中，重启后仍然有效。").TextColor(p.muted).Wrap()
				ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
					if len(s.apps) > 0 {
						options := []string{"手动输入"}
						selected := "手动输入"
						for i, app := range s.apps {
							options = append(options, app.App+" — "+app.Title)
							if normalized, valid := normalizePresetApp(s.newPresetApp); valid && normalized == app.App {
								selected = options[i+1]
							}
						}
						if ui.Select(c, &selected, options).Label("打开的窗口").Width(280).Changed() && selected != "手动输入" {
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
				ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
					ui.TextInput(c, &s.newPresetApp).Placeholder("程序名或完整路径，例如 code.exe").Width(280)
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
					ui.Text(c, s.presetError).TextColor(c.Theme().Danger).Wrap()
				}
			})
		})
		s.settingBlock(c, "general/指定软件规则列表", func() {
			remove := -1
			change := -1
			changeMode := ""
			if len(s.config.AppPresetRules) == 0 {
				ui.Text(c, "还没有指定软件规则").TextColor(colors(c).muted).Padding(14, 18)
			}
			ui.List(c, nil, len(s.config.AppPresetRules), func(i int) {
				rule := s.config.AppPresetRules[i]
				mode := rule.Mode
				ui.Row(c).Padding(11, 18).Gap(12).AlignItems(ui.Center).Children(func() {
					ui.Column(c).Grow(1).Shrink(1).Children(func() {
						ui.Text(c, rule.App).SingleLine()
						ui.Text(c, "切入时："+nameOf(presetModes, mode)).FontSize(12.5).TextColor(colors(c).muted)
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
