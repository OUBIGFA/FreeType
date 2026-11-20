package main

import (
	"fmt"
	"strings"

	"github.com/egoist/mygo/ui"
)

// 页面：ID、侧边栏名字、图标。
var pages = []struct {
	ID, Name string
	Icon     **ui.SVG
}{
	{"general", "常规", &icons.general},
	{"candidates", "候选窗与外观", &icons.candidates},
	{"typing", "拼音与按键", &icons.typing},
	{"features", "实用功能", &icons.features},
	{"shortcuts", "快捷键", &icons.shortcuts},
	{"dicts", "词库", &icons.dicts},
	{"phrases", "快捷短语", &icons.phrases},
	{"common", "常用语", &icons.common},
	{"learned", "学到的词", &icons.learned},
	{"stats", "输入统计", &icons.stats},
	{"sync", "数据与同步", &icons.sync},
	{"about", "关于", &icons.about},
}

func (s *settings) view(c *ui.Context) {
	c.SetTheme(settingsTheme(c))
	if s.notice != "" {
		c.Toast(s.notice)
		s.notice = ""
	}
	p := colors(c)
	bar := c.TitleBar()
	ui.Column(c).Fill().Background(p.page).Children(func() {
		// 系统标题栏藏起来了：这一条能拖动窗口，右上角是窗口按钮
		ui.Row(c).Height(max(bar.Height, 32)).Shrink(0).DragWindow()
		s.body(c, p)
	})
}

func (s *settings) body(c *ui.Context, p palette) {
	ui.Row(c).Grow(1).Children(func() {
		ui.Column(c).Width(236).FillHeight().Padding(4, 12, 16, 12).Gap(14).Children(func() {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Padding(2, 8).Children(func() {
				s.logo(c, 32)
				ui.Column(c).Gap(1).Children(func() {
					ui.Text(c, "FreeType").FontSize(textSize(c, 15)).FontWeight(600)
					ui.Text(c, "输入法设置").FontSize(textSize(c, 12)).TextColor(p.muted)
				})
			})
			ui.Scroll(c).Grow(1).FillWidth().Children(func() {
				ui.Column(c).Gap(2).Children(func() {
					for _, pg := range pages {
						if navItem(c, *pg.Icon, pg.Name, s.page == pg.ID) {
							if s.page != pg.ID {
								s.pageScroll.X, s.pageScroll.Y = 0, 0
							}
							s.page = pg.ID
							s.settingQuery, s.locatedSetting, s.revealSetting = "", "", false
							if pg.ID == "stats" {
								s.loadStats(false)
							}
							if pg.ID == "learned" && !s.wordsLoaded {
								s.loadWords()
							}
						}
					}
				})
			})
			s.searchField(c)
		})
		contentKey, scroll := s.page, &s.pageScroll
		if strings.TrimSpace(s.settingQuery) != "" {
			contentKey, scroll = "search", &s.searchScroll
		}
		ui.Scroll(c).Key(contentKey).TrackScroll(scroll).Grow(1).FillHeight().Padding(8, 28, 36, 28).Children(func() {
			// 宽度不变，窗口宽了左右留出一样的空
			ui.Column(c).MaxWidth(860).FillWidth().Margin(0, ui.Auto).Gap(6).Children(func() {
				if strings.TrimSpace(s.settingQuery) != "" {
					s.searchPage(c)
					return
				}
				switch s.page {
				case "candidates":
					s.candidatesPage(c)
				case "typing":
					s.typingPage(c)
				case "features":
					s.featuresPage(c)
				case "shortcuts":
					s.shortcutsPage(c)
				case "dicts":
					s.dictsPage(c)
				case "phrases":
					s.phrasesPage(c)
				case "common":
					s.commonPage(c)
				case "learned":
					s.learnedPage(c)
				case "stats":
					s.statsPage(c)
				case "sync":
					s.syncPage(c)
				case "about":
					s.aboutPage(c)
				default:
					s.generalPage(c)
				}
			})
		})
	})
}

// 按资源目录、外观和自定义内容缓存图像；切换外观或再次导入时立即换图，
// 普通重画不重复读取、解码。没有资源的离屏测试仍使用字母占位。
func (s *settings) logo(c *ui.Context, size float32) {
	box := ui.Column(c).Size(size, size).Shrink(0)
	key := struct{ resources, variant, custom string }{s.resources, s.config.BrandIcon, s.config.CustomBrandIcon}
	image := ui.Local[ui.ImageSource](box, key, func() ui.ImageSource {
		if s.resources == "" && s.config.BrandIcon != "custom" {
			return nil
		}
		data, err := loadBrandIcon(s.resources, s.config, "logo.png")
		if err == nil {
			var decoded ui.ImageSource
			decoded, err = ui.DecodeImage(data)
			if err == nil {
				return decoded
			}
		}
		s.notice = "读取图标失败：" + err.Error()
		return nil
	})
	box.Children(func() {
		if *image != nil {
			ui.Image(c, *image).Size(size, size)
			return
		}
		ui.Text(c, "F").FontSize(size*0.56).FontWeight(600).TextColor(ui.Hex("#18181b")).TextAlign(ui.Center).
			Size(size, size).Radius(controlRadius).Background(brandPink).Center()
	})
}

// statusCard 显示服务状态、默认输入法和当前输入状态。
func (s *settings) statusCard(c *ui.Context) {
	p := colors(c)
	t := c.Theme()
	card := ui.Row(c).Gap(16).Padding(18, rowInset).Radius(panelRadius).Background(p.card).Border(1, p.border).AlignItems(ui.Center)
	if !c.Preferences().HighContrast {
		alpha := float32(0.03)
		if t.Dark {
			alpha = 0.18
		}
		card.Shadow(0, 1, 3, 0, ui.RGBA(0, 0, 0, alpha))
	}
	card.Children(func() {
		s.logo(c, 44)
		ui.Column(c).Grow(1).Basis(0).MinWidth(0).Gap(4).Children(func() {
			ui.Text(c, "FreeType 输入法").FontSize(textSize(c, 16)).FontWeight(600).Wrap()
			switch {
			case s.statusErr != "":
				ui.Text(c, "无法连接输入法服务："+s.statusErr).TextColor(t.Danger).Wrap()
			case s.status.Error != "":
				ui.Text(c, "词库载入失败："+s.status.Error).TextColor(t.Danger).Wrap()
			case !s.status.Ready:
				ui.Text(c, "输入法服务正在载入词库").TextColor(p.muted)
			default:
				ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
					ui.Icon(c, statusCheck).FontSize(textSize(c, 15)).TextColor(t.Success)
					ui.Text(c, "正在运行 · 版本 "+s.version).FontSize(textSize(c, 13)).TextColor(p.muted).Wrap()
				})
			}
		})
	})
	section(c, "", func() {
		s.defaultTipRow(c)
	})
	if s.statusErr != "" || !s.status.Ready {
		return
	}
	section(c, "现在的状态", func() {
		mode := 0
		if s.status.Mode == "english" {
			mode = 1
		}
		s.settingRow(c, "general/中文 / 英文", func() {
			if ui.Segmented(c, &mode, "中文", "英文").Label("中文 / 英文").Changed() {
				s.run(op("toggle_mode"))
			}
		})
		punct := 0
		if !s.config.FullwidthPunct {
			punct = 1
		}
		s.settingRow(c, "general/标点", func() {
			if ui.Segmented(c, &punct, "中文标点", "英文标点").Label("标点").Changed() {
				s.run(command("toggle_punct"))
			}
		})
		script := 0
		if s.config.Traditional {
			script = 1
		}
		s.settingRow(c, "general/简体 / 繁体", func() {
			if ui.Segmented(c, &script, "简体", "繁體").Label("简体 / 繁体").Changed() {
				s.run(command("toggle_traditional"))
			}
		})
	})
}

func (s *settings) generalPage(c *ui.Context) {
	pageHeader(c, "常规", "")
	s.statusCard(c)
	if !s.loaded {
		return
	}
	changed := false
	section(c, "中英文切换", func() {
		s.settingRow(c, "general/切换键", func() {
			changed = choose(c, "切换键", shiftToggles, &s.config.ShiftToggle, 200) || changed
		})
		s.settingRow(c, "general/默认状态", func() {
			changed = choose(c, "默认状态", modes, &s.config.DefaultMode, 200) || changed
		})
		if s.config.DefaultMode != "global" || s.showSetting("general/中英文状态") {
			s.settingRow(c, "general/中英文状态", func() {
				changed = choose(c, "中英文状态", modeScopes, &s.config.ModeScope, 200) || changed
			})
		}
	})
	section(c, "状态提示", func() {
		changed = s.settingSwitch(c, "general/显示托盘图标", false, &s.config.TrayIcon) || changed
		changed = s.settingSwitch(c, "general/悬浮状态栏", false, &s.config.StatusBar) || changed
		if s.config.StatusBar || s.showSetting("general/自动隐藏") {
			changed = s.settingSwitch(c, "general/自动隐藏", true, &s.config.StatusBarAutohide) || changed
		}
		if s.config.StatusBar {
			changed = s.statusBarAppearance(c) || changed
		}
		changed = s.settingSwitch(c, "general/切换状态时提示", false, &s.config.HintOnToggle) || changed
		changed = s.settingSwitch(c, "general/切换输入框时提示", false, &s.config.HintOnFocus) || changed
		s.settingRow(c, "general/提示位置", func() {
			changed = choose(c, "提示位置", positions, &s.config.HintPosition, 200) || changed
		})
	})
	if changed {
		s.saveConfig()
	}
}

func (s *settings) statusBarAppearance(c *ui.Context) (changed bool) {
	for _, field := range []struct {
		title     string
		value     *int
		low, high float64
	}{
		{"缩放比例 / 百分比", &s.config.BarScale, 75, 150}, {"图标字号", &s.config.BarIconSize, 12, 23}, {"自动隐藏延时 / 秒", &s.config.BarHideSeconds, 1, 60},
	} {
		if field.title == "自动隐藏延时 / 秒" && !s.config.StatusBarAutohide {
			continue
		}
		subRow(c, field.title, "", func() {
			value := float64(*field.value)
			if ui.NumberInput(c, &value, field.low, field.high, 1).Label(field.title).Width(120).Changed() {
				*field.value = int(value)
				changed = true
			}
		})
	}
	changed = switchLine(c, "阴影", "", &s.config.BarShadow, true) || changed
	subRow(c, "显示组件", "", func() {
		ui.Row(c).Wrap().Gap(10).Children(func() {
			for i, label := range []string{"标点", "简繁", "Windows 表情", "设置"} {
				bit := 1 << i
				on := s.config.BarParts&bit != 0
				if checkbox(c, &on, label).Changed() {
					s.config.BarParts ^= bit
					changed = true
				}
			}
		})
	})
	return changed
}

func (s *settings) candidatesPage(c *ui.Context) {
	pageHeader(c, "候选窗与外观", "调整候选窗显示和 FreeType 品牌图标")
	if !s.loaded {
		s.candidatePreview(c)
		return
	}
	changed := false
	s.candidatePreview(c)
	section(c, "外观", func() {
		s.settingRow(c, "candidates/字体", func() {
			family := s.config.FontFamily
			ui.Column(c).Children(func() {
				if ui.Combobox(c, &family, s.fontList(fontNames)).Label("字体").Width(240).MaxWidthPercent(100).Shrink(1).Changed() && family != "" {
					s.config.FontFamily = family
					changed = true
				}
			})
			if s.config.FontFamily != "Microsoft YaHei" && ui.Button(c, "恢复默认").Clicked() {
				s.config.FontFamily = "Microsoft YaHei"
				changed = true
			}
		})
		s.settingRow(c, "candidates/字号", func() {
			size := float64(s.config.FontSize)
			ui.Column(c).Children(func() {
				if ui.NumberInput(c, &size, 12, 36, 1).Label("字号").Width(120).Changed() {
					s.config.FontSize = int(size)
					changed = true
				}
			})
		})
		s.settingRow(c, "candidates/候选强调色", func() {
			changed = choose(c, "候选强调色", candidateStyleChoices, &s.config.CandidateStyle, 220) || changed
		})
		s.settingRow(c, "candidates/每页候选个数", func() {
			size := fmt.Sprint(s.config.PageSize)
			if ui.Select(c, &size, pageSizes).Label("每页候选个数").Width(120).Changed() {
				fmt.Sscan(size, &s.config.PageSize)
				changed = true
			}
		})
		changed = s.settingSwitch(c, "candidates/滚轮翻页", false, &s.config.CandidateWheel) || changed
		s.settingRow(c, "candidates/固定排位角标", func() {
			changed = choose(c, "固定排位角标", []choice{{"paperclip", "回形针"}, {"pushpin", "图钉"}, {"dot", "圆点"}, {"off", "不显示"}}, &s.config.PinBadge, 140) || changed
		})
		s.settingRow(c, "candidates/排列方向", func() {
			dir := 0
			if !s.config.Horizontal {
				dir = 1
			}
			if ui.Segmented(c, &dir, "横排", "竖排").Label("排列方向").Changed() {
				s.config.Horizontal = dir == 0
				changed = true
			}
		})
	})
	section(c, "预编辑与位置", func() {
		s.settingRow(c, "candidates/行内预编辑", func() {
			changed = choose(c, "行内预编辑", []choice{{"segmented", "拼音分词"}, {"raw", "原始按键"}, {"off", "不显示"}}, &s.config.InlinePreedit, 160) || changed
		})
		changed = s.settingSwitch(c, "candidates/候选窗预编辑", false, &s.config.CandidatePreedit) || changed
		s.settingRow(c, "candidates/预编辑字号", func() {
			size := float64(s.config.PreeditSize)
			if size == 0 {
				size = float64(s.config.FontSize) * 0.78
			}
			if ui.NumberInput(c, &size, 10, 36, 1).Width(110).Label("预编辑字号").Changed() {
				s.config.PreeditSize = int(size)
				changed = true
			}
			if s.config.PreeditSize != 0 && ui.Button(c, "自动").Clicked() {
				s.config.PreeditSize = 0
				changed = true
			}
		})
		changed = s.settingSwitch(c, "candidates/跟随光标", false, &s.config.CandidateFollow) || changed
		changed = s.settingSwitch(c, "candidates/横排保持单行", false, &s.config.CandidateSingleLine) || changed
		s.settingRow(c, "candidates/补充字体", func() {
			if ui.TextInput(c, &s.config.FontFallbacks).Label("补充字体").Placeholder("Microsoft YaHei, SimSun").Width(220).MaxWidthPercent(100).Changed() {
				changed = true
			}
		})
	})
	section(c, "品牌外观", func() {
		s.settingRow(c, "candidates/品牌图标", func() {
			s.logo(c, 32)
			choices := []choice{{"default", "默认"}, {"white", "白色"}}
			if s.config.CustomBrandIcon != "" {
				choices = append(choices, choice{"custom", "自定义"})
			}
			changed = choose(c, "品牌图标", choices, &s.config.BrandIcon, 140) || changed
			if ui.Button(c, "自定义").Disabled(s.brandIconBusy).Clicked() {
				s.chooseBrandIcon()
			}
		})
	})
	hint(c, "任务栏图标需重启资源管理器后生效")
	if changed {
		s.saveConfig()
	}
}

// selectedLook 是候选窗里选中候选的样子。
type selectedLook struct {
	block bool     // 画底色块；搜狗样式不画
	fill  ui.Color // 底色块的颜色
	text  ui.Color // 选中候选的字（和序号）
}

// candidateStyle 和候选窗（windows/tsf/src/ui.rs 的 theme）用一样的颜色，改一边要改另一边。
func candidateStyle(id string, dark bool) selectedLook {
	switch id {
	case "blue":
		if dark {
			return selectedLook{true, ui.Hex("#3d7ff0"), ui.Hex("#18181b")}
		}
		return selectedLook{true, ui.Hex("#2b6ce6"), ui.Hex("#ffffff")}
	case "green":
		// 微信绿略微加深，让浅色与深色外观中的白字更清楚。
		return selectedLook{true, ui.Hex("#03d468"), ui.Hex("#ffffff")}
	case "sogou":
		if dark {
			return selectedLook{false, ui.Color{}, ui.Hex("#6c9eff")}
		}
		return selectedLook{false, ui.Color{}, ui.Hex("#1d66df")}
	default:
		return selectedLook{true, brandPink, ui.Hex("#18181b")}
	}
}

// candidatePreview 按现在的字体、字号、排列和强调色画一个候选窗的样子。
func (s *settings) candidatePreview(c *ui.Context) {
	p := colors(c)
	t := c.Theme()
	family, size := s.config.FontFamily, textSize(c, float32(s.config.FontSize))
	all := []string{"你好", "拟好", "你", "泥", "妮", "逆", "呢", "倪", "腻", "溺"}
	words := all[:min(max(s.config.PageSize, 1), len(all))]
	ui.Column(c).FillWidth().Gap(8).Margin(4, 0, 0, 0).Children(func() {
		ui.Text(c, "预览").FontSize(textSize(c, 13)).FontWeight(600).TextColor(p.muted).Padding(0, 4)
		// 外层画安静的预览舞台，横排按真实字号绘制；超出可用宽度时在预览内滚动，不压缩候选词。
		ui.Column(c).FillWidth().Padding(16, rowInset, 12, rowInset).Radius(panelRadius).
			Background(p.subtle).Border(1, p.border).Children(func() {
			// 外层用可视宽度对齐；内层按候选宽度收起，放不下时才缩到可视宽度。
			ui.Row(c).FillWidth().Justify(ui.Center).Label("候选预览区域").Children(func() {
				ui.ScrollHorizontal(c).MinWidth(0).Shrink(1).Padding(2, 2, 10, 2).Children(func() {
					popup := ui.Column(c).Padding(8, 10).Radius(panelRadius).Background(p.card).Border(1, p.border).
						Label("候选预览窗口").AlignSelf(ui.Start).Shrink(0).Gap(4)
					if !c.Preferences().HighContrast {
						alpha := float32(0.06)
						if t.Dark {
							alpha = 0.28
						}
						popup.Shadow(0, 2, 8, 0, ui.RGBA(0, 0, 0, alpha))
					}
					popup.Children(func() {
						if s.config.CandidatePreedit {
							preedit := "ni hao"
							if s.config.Shuangpin && s.shuangpinExample != "" {
								preedit = s.shuangpinExample
							}
							preeditSize := size * 0.78
							if s.config.PreeditSize > 0 {
								preeditSize = textSize(c, float32(s.config.PreeditSize))
							}
							ui.Text(c, preedit).Font(family).FontSize(preeditSize).TextColor(p.muted).Padding(0, 4)
						}
						list := ui.Row(c).Gap(2)
						if !s.config.Horizontal {
							list = ui.Column(c).Gap(2)
						}
						list.Children(func() {
							defer func() {
								// 和真的候选窗一样，最后是翻页箭头
								ui.Row(c).Gap(6).Padding(3, 8).Children(func() {
									ui.Text(c, "‹").Font(family).FontSize(size).TextColor(p.muted)
									ui.Text(c, "›").Font(family).FontSize(size).TextColor(t.Text)
								})
							}()
							style := candidateStyle(s.config.CandidateStyle, t.Dark)
							for i, w := range words {
								item := ui.Row(c).Gap(4).Padding(4, 8).Radius(controlRadius).AlignItems(ui.Center).Shrink(0)
								label, text := p.muted, t.Text
								if i == 0 {
									if style.block {
										item.Background(style.fill)
									}
									label, text = style.text, style.text
								}
								item.Children(func() {
									ui.Text(c, fmt.Sprint((i+1)%10)).Font(family).FontSize(size * 0.78).TextColor(label)
									ui.Text(c, w).Font(family).FontSize(size).TextColor(text)
								})
							}
						})
					})
				})
			})
		})
	})
}

func (s *settings) typingPage(c *ui.Context) {
	pageHeader(c, "拼音与按键", "")
	if !s.loaded {
		return
	}
	changed := false
	section(c, "拼音", func() {
		changed = s.settingSwitch(c, "typing/双拼", false, &s.config.Shuangpin) || changed
		if s.config.Shuangpin || s.showSetting("typing/双拼方案") || s.showSetting("typing/双拼展示键位") {
			s.settingSubRow(c, "typing/双拼方案", func() {
				changed = choose(c, "双拼方案", schemes, &s.config.ShuangpinScheme, 180) || changed
			})
			changed = s.settingSwitch(c, "typing/双拼展示键位", true, &s.config.ShuangpinShowKeys) || changed
		}
		changed = s.settingSwitch(c, "typing/模糊音", false, &s.config.Fuzzy) || changed
		if s.config.Fuzzy || s.showSetting("typing/模糊音分组") {
			s.settingBlock(c, "typing/模糊音分组", func() {
				ui.Column(c).FillWidth().Padding(2, 12, 6, 12).Children(func() {
					ui.Row(c).FillWidth().Wrap().GapX(16).GapY(12).Padding(12, rowInset-12, 12, subInset-12).Radius(controlRadius).Background(colors(c).subtle).Children(func() {
						for _, rule := range fuzzyRules {
							on := s.config.FuzzyGroups&rule.Bit != 0
							if checkbox(c, &on, rule.Label).Width(120).Changed() {
								if on {
									s.config.FuzzyGroups |= rule.Bit
								} else {
									s.config.FuzzyGroups &^= rule.Bit
								}
								changed = true
							}
						}
					})
				})
			})
		}
		changed = s.settingSwitch(c, "typing/按错键纠错", false, &s.config.TypoCorrection) || changed
		if s.config.TypoCorrection || s.showSetting("typing/纠错类型") {
			s.settingRow(c, "typing/纠错类型", func() {
				ui.Row(c).Wrap().GapX(14).GapY(8).Children(func() {
					for index, label := range []string{"乱序", "邻键", "漏字", "多字"} {
						bit := 1 << index
						on := s.config.TypoKinds&bit != 0
						if checkbox(c, &on, label).Changed() {
							s.config.TypoKinds ^= bit
							changed = true
						}
					}
				})
			})
		}
		if s.config.TypoCorrection || s.showSetting("typing/纠错标记") {
			changed = s.settingSwitch(c, "typing/纠错标记", true, &s.config.TypoMark) || changed
		}
		changed = s.settingSwitch(c, "typing/显示更多生僻字", false, &s.config.RareChars) || changed
	})
	section(c, "选词与翻页", func() {
		s.settingRow(c, "typing/翻页键", func() {
			for _, k := range []struct {
				bit   int
				label string
			}{{pageKeyMinus, "- ="}, {pageKeyBracket, "[ ]"}, {pageKeyComma, ", ."}, {pageKeyTab, "Tab"}} {
				on := s.config.PageKeys&k.bit != 0
				if checkbox(c, &on, k.label).Changed() {
					if on {
						s.config.PageKeys |= k.bit
					} else {
						s.config.PageKeys &^= k.bit
					}
					changed = true
				}
			}
		})
		s.settingRow(c, "typing/分号、引号选词", func() {
			changed = choose(c, "分号、引号选词", selectKeys, &s.config.SelectKeys, 220) || changed
		})
		s.settingRow(c, "typing/左右方向键", func() {
			changed = choose(c, "左右方向键", arrowKeys, &s.config.ArrowKeys, 200) || changed
		})
		changed = s.settingSwitch(c, "typing/以词定字", false, &s.config.PickChar) || changed
	})
	section(c, "标点", func() {
		changed = s.settingSwitch(c, "typing/中文标点", false, &s.config.FullwidthPunct) || changed
		showDigitHalf := s.showSetting("typing/数字标点保持半角")
		showDigitExcept := s.showSetting("typing/数字后例外保持全角")
		showDigitSymbols := s.showSetting("typing/数字后保持全角的标点")
		if s.config.FullwidthPunct || showDigitHalf || showDigitExcept || showDigitSymbols {
			changed = s.settingSwitch(c, "typing/数字标点保持半角", true, &s.config.DigitHalfwidthPunct) || changed
			if (s.config.FullwidthPunct && s.config.DigitHalfwidthPunct) || showDigitExcept || showDigitSymbols {
				changed = s.settingSwitch(c, "typing/数字后例外保持全角", true, &s.config.DigitPunctExceptions) || changed
				if (s.config.FullwidthPunct && s.config.DigitHalfwidthPunct && s.config.DigitPunctExceptions) || showDigitSymbols {
					s.settingBlock(c, "typing/数字后保持全角的标点", func() {
						ui.Column(c).FillWidth().Padding(2, 12, 6, 12).Children(func() {
							ui.Row(c).FillWidth().Wrap().GapX(16).GapY(12).Padding(12, rowInset-12, 12, subInset-12).Radius(controlRadius).Background(colors(c).subtle).Children(func() {
								for _, rule := range digitPunctRules {
									on := s.config.DigitPunctFullwidth&rule.Bit != 0
									if checkbox(c, &on, rule.Label).Width(max(132, textSize(c, 132))).Changed() {
										if on {
											s.config.DigitPunctFullwidth |= rule.Bit
										} else {
											s.config.DigitPunctFullwidth &^= rule.Bit
										}
										changed = true
									}
								}
							})
						})
					})
				}
			}
		}
		changed = s.settingSwitch(c, "typing/双引号改用直角引号", false, &s.config.CornerQuotes) || changed
		changed = s.settingSwitch(c, "typing/成对标点自动补全", false, &s.config.AutoPair) || changed
		changed = s.settingSwitch(c, "typing/小键盘的点输入「.」", false, &s.config.NumpadDot) || changed
	})
	section(c, "中英文混排", func() {
		changed = s.settingSwitch(c, "typing/中英文之间自动加空格", false, &s.config.Autospace) || changed
		if s.config.Autospace || s.showSetting("typing/汉字和字母之间") || s.showSetting("typing/汉字和数字之间") {
			changed = s.settingSwitch(c, "typing/汉字和字母之间", true, &s.config.AutospaceLetters) || changed
			changed = s.settingSwitch(c, "typing/汉字和数字之间", true, &s.config.AutospaceDigits) || changed
		}
	})
	if changed {
		s.saveConfig()
	}
}

func (s *settings) featuresPage(c *ui.Context) {
	pageHeader(c, "实用功能", "")
	if !s.loaded {
		return
	}
	changed := false
	section(c, "工具栏与表情", func() {
		changed = s.settingSwitch(c, "features/v + 空格打开工具栏", false, &s.config.Toolbar) || changed
		changed = s.settingSwitch(c, "features/记录剪贴板历史", false, &s.config.ClipboardHistory) || changed
		if s.config.ClipboardHistory || s.showSetting("features/清空剪贴板历史") {
			s.settingSubRow(c, "features/清空剪贴板历史", func() {
				if ui.Button(c, "清空").Clicked() {
					s.clearClipboard()
				}
			})
		}
		changed = s.settingSwitch(c, "features/推荐 emoji", false, &s.config.EmojiSuggest) || changed
	})
	section(c, "智能输入", func() {
		s.neuralModelRow(c)
		changed = s.settingSwitch(c, "features/长词前缀补全", false, &s.config.LongWordCompletion) || changed
		changed = s.settingSwitch(c, "features/邮箱、网址补全", false, &s.config.EmailCompletion) || changed
		changed = s.settingSwitch(c, "features/日期时间", false, &s.config.DateTime) || changed
		changed = s.settingSwitch(c, "features/V 模式：计算与数字大写", false, &s.config.Calculator) || changed
		changed = s.settingSwitch(c, "features/快捷短语混入候选", false, &s.config.QuickPhrases) || changed
		changed = s.settingSwitch(c, "features/输出繁体", false, &s.config.Traditional) || changed
	})
	section(c, "联想", func() {
		changed = s.settingSwitch(c, "features/上屏联想", false, &s.config.PredictNext) || changed
		if s.config.PredictNext || s.showSetting("features/数字键选联想词") || s.showSetting("features/空格选联想词") || s.showSetting("features/左右方向键选联想词") || s.showSetting("features/双击空格键选词") {
			changed = s.settingSwitch(c, "features/数字键选联想词", true, &s.config.PredictDigits) || changed
			changed = s.settingSwitch(c, "features/空格选联想词", true, &s.config.PredictSpace) || changed
			if s.config.PredictSpace || s.showSetting("features/双击空格键选词") {
				ui.Column(c).FillWidth().Padding(0, 0, 0, 20).Children(func() {
					changed = s.settingSwitch(c, "features/双击空格键选词", true, &s.config.PredictDoubleSpace) || changed
				})
			}
			changed = s.settingSwitch(c, "features/左右方向键选联想词", true, &s.config.PredictArrows) || changed
		}
	})
	if s.config.PredictNext {
		hint(c, "联想词来自内置语言模型和学习记录；常用搭配优先显示；搭配记录随学习备份导入导出")
	}
	s.appPresetSection(c)
	if changed {
		s.saveConfig()
	}
}

func (s *settings) shortcutsPage(c *ui.Context) {
	pageHeader(c, "快捷键", "")
	if !s.loaded {
		return
	}
	changed := false
	section(c, "可以设置的快捷键", func() {
		s.settingRow(c, "shortcuts/表情面板", func() {
			changed = choose(c, "表情面板", hotkeysEmoji, &s.config.HotkeyEmoji, 180) || changed
		})
		s.settingRow(c, "shortcuts/中文 / 英文标点", func() {
			changed = choose(c, "中文 / 英文标点", hotkeysPunct, &s.config.HotkeyPunct, 180) || changed
		})
		s.settingRow(c, "shortcuts/简体 / 繁体", func() {
			changed = choose(c, "简体 / 繁体", hotkeysTraditional, &s.config.HotkeyTraditional, 180) || changed
		})
		s.settingRow(c, "shortcuts/切换中文 / 英文", func() {
			changed = choose(c, "切换中文 / 英文", hotkeysMode, &s.config.HotkeyMode, 180) || changed
		})
	})
	hint(c, "快捷键默认关闭；启用后，FreeType 会先接收该按键；应用中的同名快捷键将无法使用")
	section(c, "固定按键", func() {
		s.settingBlock(c, "shortcuts/固定按键", func() {
			for _, k := range [][2]string{
				{"Shift", "切换中英文；可在常规中修改"},
				{"空格 / 1–9 / 0", "空格选择当前候选；数字选择对应候选；每页有 10 项时，0 选择第 10 项；回车上屏原始输入"},
				{"- =  PgUp PgDn", "翻页；其他翻页键可在「拼音与按键」中设置"},
				{"← →  ↓", "移动候选；横排时 ↓ 展开更多候选"},
				{"Ctrl + Delete", "删除当前候选的学习记录；也可长按或右键候选"},
				{"[ ]", "启用「以词定字」后，选择候选的第一个 / 最后一个字；设置为翻页键时用于翻页"},
				{"v + 空格", "启用工具栏后，在全拼状态下打开剪贴板、常用语、符号和表情"},
				{"Esc", "清空当前输入，或关闭工具栏、表情面板和上屏联想"},
			} {
				ui.Row(c).Padding(10, 0).Gap(16).Wrap().AlignItems(ui.Center).Children(func() {
					ui.Row(c).Width(172).MaxWidthPercent(100).Wrap().Gap(6).Children(func() {
						for _, part := range strings.Split(k[0], "  ") {
							keyCap(c, part)
						}
					})
					ui.Text(c, k[1]).FontSize(textSize(c, 13)).TextColor(colors(c).muted).Grow(1).Basis(200).MinWidth(160).LineHeight(1.45).Wrap()
				})
			}
		})
	})
	if changed {
		s.saveConfig()
	}
}

func (s *settings) reportView(c *ui.Context) {
	if s.report == "" {
		return
	}
	ui.Column(c).Gap(4).Margin(10, 0, 0, 0).Children(func() {
		ui.Text(c, s.report).FontWeight(600).Wrap()
		for _, e := range s.reportErrors {
			ui.Text(c, e).FontSize(textSize(c, 13)).TextColor(c.Theme().Danger).LineHeight(1.4).Wrap()
		}
	})
}

func (s *settings) dictsPage(c *ui.Context) {
	pageHeader(c, "词库", "导入词库参与组词和排序；与学习记录分开保存")
	section(c, "主词库", func() {
		current := "默认；白霜＋万象＋项目热词"
		if s.config.Lexicon != "" {
			current = s.config.Lexicon
		}
		s.settingRowValue(c, "dicts/词库文件", current, func() {
			if ui.Button(c, "选择文件").Label("选择主词库文件").Clicked() && s.openTable != nil {
				s.background(func() func() {
					path := s.openTable("选择主词库 TSV 文件")
					if path == "" {
						return nil
					}
					return func() { s.chooseLexicon(path) }
				})
			}
			if s.config.Lexicon != "" && ui.Button(c, "恢复默认").Label("恢复默认词库").Clicked() {
				s.chooseLexicon("")
			}
		})
	})
	hint(c, "默认词库包含白霜、万象补充和项目热词；自定义主词库使用 TSV，每行依次填写拼音、词语和词频，用制表符分隔；选择后在后台载入，文件不存在时使用默认词库；其他格式请使用下方的导入词库")
	section(c, "导入词库", func() {
		s.settingRow(c, "dicts/导入词库", func() {
			if ui.PrimaryButton(c, "导入词库").Label("导入词库文件").Clicked() {
				s.importDictionary()
			}
		})
	})
	s.reportView(c)
	section(c, "已导入", func() {
		s.settingBlock(c, "dicts/已导入词库", func() {
			if len(s.dicts) == 0 {
				row(c, "还没有导入词库", "", nil)
				return
			}
			for _, d := range s.dicts {
				name := d.Name
				row(c, name, fmt.Sprintf("%d 条", d.Entries), func() {
					if ui.Button(c, "删除").Label("删除 " + name).Clicked() {
						s.removeDictionary(name)
					}
				})
			}
		})
	})
}

func (s *settings) phrasesPage(c *ui.Context) {
	pageHeader(c, "快捷短语", "启用实用功能中的快捷短语候选后，输入完整编码即可选用；编码仅含英文字母")
	section(c, "添加", func() {
		s.settingBlock(c, "phrases/添加快捷短语", func() {
			ui.Column(c).Padding(12, 0, 14, 0).Gap(14).Children(func() {
				ui.Row(c).Gap(12).Wrap().AlignItems(ui.Start).Children(func() {
					ui.Field(c, "编码", func() { ui.TextInput(c, &s.newCode).Placeholder("例如 dz").FillWidth() }).
						Grow(1).Basis(220).MinWidth(160)
					ui.Field(c, "权重", func() { ui.TextInput(c, &s.newWeight).Placeholder("10").FillWidth() }).
						Description("同一编码有多条时，权重大的在前").Grow(1).Basis(220).MinWidth(160)
				})
				ui.Field(c, "短语", func() { ui.TextArea(c, &s.newText).Height(72) }).Error(s.formError)
				ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
					if ui.PrimaryButton(c, "添加").Disabled(s.phrasesBusy).Clicked() {
						s.addPhrase()
					}
				})
			})
		})
	})
	section(c, "批量导入、导出", func() {
		s.settingRow(c, "phrases/导入或导出", func() {
			checkbox(c, &s.replacePhrases, "替换现有快捷短语")
			if ui.Button(c, "导入").Disabled(s.phrasesBusy).Clicked() {
				s.importPhrases()
			}
			if ui.Button(c, "导出").Clicked() {
				s.export("export_quick_phrases", "导出快捷短语", "FreeType 快捷短语.txt")
			}
		})
	})
	s.reportView(c)
	ui.Row(c).Margin(18, 0, 0, 0).Padding(0, 4).Gap(12).Wrap().AlignItems(ui.Center).Children(func() {
		ui.Text(c, fmt.Sprintf("快捷短语 %d 条", len(s.phrases))).FontSize(textSize(c, 13)).FontWeight(600).TextColor(colors(c).muted).Grow(1).Basis(180).Wrap()
		ui.SearchField(c, &s.phraseQuery).Label("搜索编码或短语").Placeholder("搜索编码或短语").Width(240).MaxWidthPercent(100).Shrink(1)
	})
	section(c, "", func() {
		s.settingBlock(c, "phrases/全部快捷短语", func() {
			shown := s.filteredPhrases()
			if len(shown) == 0 {
				title := "没有快捷短语"
				if strings.TrimSpace(s.phraseQuery) != "" {
					title = "没有匹配的快捷短语"
				}
				row(c, title, "在上方添加短语，或导入已有文件", nil)
				return
			}
			remove := -1
			// 最多显示 12 行，多出来的在列表里滚动
			ui.List(c, nil, len(shown), func(k int) {
				i := shown[k]
				ph := s.phrases[i]
				item := ui.Row(c).Height(phraseRow).Padding(0, 0).Gap(12).AlignItems(ui.Center)
				if item.Hovered() {
					item.Background(colors(c).cardHover)
				}
				item.Children(func() {
					ui.Row(c).Width(96).Shrink(0).Children(func() { keyCap(c, ph.Code).Tooltip(ph.Code) })
					ui.Text(c, preview(ph.Text)).Grow(1).Basis(0).MinWidth(0).Shrink(1).SingleLine().Tooltip(ph.Text)
					ui.Textf(c, "%d", ph.Weight).Width(56).TextAlign(ui.End).FontFeatures("tnum").TextColor(colors(c).muted).FontSize(textSize(c, 13))
					if ui.Button(c, "删除").Label(fmt.Sprintf("删除 %s %d", ph.Code, i+1)).Disabled(s.phrasesBusy).Clicked() {
						remove = i
					}
				})
			}).MaxHeight(phraseRow * maxPhraseRows)
			if remove >= 0 {
				s.removePhrase(remove)
			}
		})
	})
}

func (s *settings) learnedPage(c *ui.Context) {
	pageHeader(c, "学到的词", "选词次数和搭配影响候选排序；可在此删除学习记录，也可长按候选或按 Ctrl + Delete 删除")
	section(c, "", func() {
		s.settingRow(c, "learned/学习备份", func() {
			if ui.Button(c, "导入").Clicked() {
				s.importLearning()
			}
			if ui.Button(c, "导出").Clicked() {
				s.export("export_learning", "导出学习备份", "FreeType 学习备份.json")
			}
		})
		s.settingRow(c, "learned/全部清空", func() {
			if ui.Button(c, "全部清空").Label("清空全部学习记录").Clicked() {
				s.confirmClear = true
			}
		})
	})
	s.unlockDialog(c)
	section(c, "添加词", func() {
		s.settingBlock(c, "learned/添加词", func() {
			ui.Column(c).Padding(12, 0, 14, 0).Gap(14).Children(func() {
				ui.Text(c, "中文词可留空拼音，由 FreeType 自动补充；多音字请填写拼音；英文词默认用小写单词作为输入码；中英混输词需填写输入码，例如 datag → 打 tag").FontSize(textSize(c, 13)).TextColor(colors(c).muted).LineHeight(1.45).Wrap()
				ui.Row(c).Gap(12).Wrap().AlignItems(ui.End).Children(func() {
					ui.Field(c, "词语", func() {
						ui.TextInput(c, &s.newWord).Placeholder("词语，例如 霜频阁").Label("词语").FillWidth()
					}).Width(200).MaxWidthPercent(100)
					ui.Field(c, "拼音或输入码", func() {
						ui.TextInput(c, &s.newPinyin).Placeholder("如 shuang pin ge 或 datag").Label("拼音或输入码").FillWidth()
					}).Grow(1).Basis(200).MinWidth(180)
					if ui.PrimaryButton(c, "添加").Label("添加词").Clicked() {
						s.addWord()
					}
				})
				if s.wordError != "" {
					ui.Text(c, s.wordError).FontSize(textSize(c, 13)).TextColor(c.Theme().Danger).LineHeight(1.4).Wrap()
				}
			})
		})
	})
	switch ui.AlertDialog(c, &s.confirmClear, "清空学习记录", "删除全部学习词条、选词次数和搭配；需导入学习备份才能恢复", "取消", "清空") {
	case 1:
		s.clearLearned()
	}
	words := s.filteredWords()
	ui.Row(c).Margin(18, 0, 0, 0).Padding(0, 4).Gap(12).Wrap().AlignItems(ui.Center).Children(func() {
		ui.Text(c, fmt.Sprintf("学到的词 %d 条", len(s.words))).FontSize(textSize(c, 13)).FontWeight(600).TextColor(colors(c).muted).Grow(1).Basis(180).Wrap()
		if ui.Button(c, "刷新").Clicked() {
			s.loadWords()
		}
		ui.SearchField(c, &s.wordQuery).Label("搜索词、拼音或输入码").Placeholder("搜索词、拼音或输入码").Width(240).MaxWidthPercent(100).Shrink(1)
	})
	p := colors(c)
	section(c, "", func() {
		s.settingBlock(c, "learned/词条列表", func() {
			if !s.wordsLoaded {
				row(c, "正在读取", "", nil)
				return
			}
			if len(words) == 0 {
				title := "没有学到的词"
				if strings.TrimSpace(s.wordQuery) != "" {
					title = "没有匹配的学习词条"
				}
				row(c, title, "", nil)
				return
			}
			ui.List(c, nil, len(words), func(i int) {
				w := words[i]
				item := ui.Row(c).Padding(10, 0).MinHeight(54).Gap(12).AlignItems(ui.Center)
				if item.Hovered() {
					item.Background(p.cardHover)
				}
				item.Children(func() {
					ui.Text(c, w.text()).FontWeight(500).Width(140).SingleLine().Tooltip(w.text())
					ui.Text(c, w.Pinyin).Grow(1).Basis(0).MinWidth(0).SingleLine().TextColor(p.muted).Tooltip(w.Pinyin)
					ui.Textf(c, "%.0f", w.Freq).FontSize(textSize(c, 13)).TextColor(p.muted).Width(64).TextAlign(ui.End).FontFeatures("tnum")
					if ui.Button(c, "删除").Label("删除 " + w.text()).Clicked() {
						s.forgetWords([]string{w.Word})
					}
				})
			}).MaxHeight(460)
		})
	})
}

// 快捷短语列表一行多高、最多显示几行（多出来的在列表里滚动）。
const (
	phraseRow     = 56
	maxPhraseRows = 12
)

// fontNames 是系统里能用的字体（测试里换掉）。
var fontNames = func() []string {
	var out []string
	for _, f := range ui.FontFamilies() {
		out = append(out, f.Name)
	}
	return out
}
