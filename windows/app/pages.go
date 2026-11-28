package main

import (
	"fmt"
	"math"
	"strconv"
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
	ui.Row(c).Grow(1).Disabled(s.resetting).Children(func() {
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
				ui.Text(c, s.status.Error).TextColor(t.Danger).Wrap()
			case !s.status.Ready:
				ui.Text(c, "输入法服务正在载入词库").TextColor(p.muted)
			default:
				ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
					ui.Icon(c, statusCheck).FontSize(textSize(c, 15)).TextColor(t.Success)
					ui.Text(c, "正在运行 · 版本 "+s.version).FontSize(textSize(c, 13)).TextColor(p.muted).Wrap()
				})
				if s.status.LexiconWarning != "" {
					ui.Text(c, s.status.LexiconWarning).Wrap()
				}
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
		settingGroup(c, func() {
			s.settingRow(c, "general/默认状态", func() {
				changed = choose(c, "默认状态", modes, &s.config.DefaultMode, 200) || changed
			})
		}, s.config.DefaultMode != "global" || s.showSetting("general/中英文状态"), func() {
			s.settingSubRow(c, "general/中英文状态", func() {
				changed = choose(c, "中英文状态", modeScopes, &s.config.ModeScope, 200) || changed
			})
		})
	})
	section(c, "状态提示", func() {
		changed = s.settingSwitch(c, "general/显示托盘图标", false, &s.config.TrayIcon) || changed
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "general/悬浮状态栏", false, &s.config.StatusBar) || changed
		}, s.config.StatusBar || s.showSetting("general/自动隐藏"), func() {
			ui.Column(c).FillWidth().Children(func() {
				changed = s.settingSwitch(c, "general/自动隐藏", true, &s.config.StatusBarAutohide) || changed
				if s.config.StatusBar && s.config.StatusBarAutohide {
					nestedSubGroup(c, func() {
						subRow(c, "自动隐藏延时 / 秒", "", func() {
							value := float64(s.config.BarHideSeconds)
							if ui.NumberInput(c, &value, 1, 60, 1).Label("自动隐藏延时 / 秒").Width(120).Changed() {
								s.config.BarHideSeconds = int(value)
								changed = true
							}
						})
					})
				}
			})
			if s.config.StatusBar {
				changed = s.statusBarAppearance(c) || changed
			}
		})
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
		{"缩放比例 / 百分比", &s.config.BarScale, 75, 150}, {"图标字号", &s.config.BarIconSize, 12, 23},
	} {
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
			for _, part := range []struct {
				label string
				bit   int
			}{{"品牌标识（设置）", 8}, {"标点", 1}, {"简繁", 2}, {"Windows 表情", 4}} {
				label, bit := part.label, part.bit
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
	colorChanged := false
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
		settingGroup(c, func() {
			s.settingRow(c, "candidates/候选强调色", func() {
				changed = choose(c, "候选皮肤", candidateStyleChoices, &s.config.CandidateStyle, 220) || changed
			})
		}, s.config.CandidateStyle == "custom" || s.showSetting("candidates/选中底色块") || s.showSetting("candidates/自定义配色"), func() {
			ui.Column(c).FillWidth().Children(func() {
				changed = s.settingSwitch(c, "candidates/选中底色块", true, &s.config.CandidateSkin.AccentFill) || changed
				s.settingBlock(c, "candidates/自定义配色", func() {
					subContent(c, func() {
						p := colors(c)
						ui.Text(c, "选择预设配色快速开始，或单独调整下方各部分颜色").FontSize(textSize(c, 12.5)).TextColor(p.muted).Wrap()
						ui.Row(c).FillWidth().Wrap().GapX(8).GapY(8).AlignItems(ui.Center).Children(func() {
							for _, tpl := range candidateSkinTemplates {
								active := sameCandidateSkin(s.config.CandidateSkin, tpl.skin)
								if skinPresetChip(c, tpl.name, tpl.skin, active) {
									s.config.CandidateSkin = tpl.skin
									changed = true
								}
							}
							if ui.Button(c, "恢复默认配色").Clicked() {
								s.config.CandidateSkin = defaultCandidateSkin()
								changed = true
							}
						})
						d := defaultCandidateSkin()
						type skinColorField struct {
							label, fallback string
							field           *string
						}
						fields := []skinColorField{
							{"窗口背景", d.Background, &s.config.CandidateSkin.Background},
							{"窗口边框", d.Border, &s.config.CandidateSkin.Border},
							{"候选文字", d.Text, &s.config.CandidateSkin.Text},
							{"拼音与序号", d.Muted, &s.config.CandidateSkin.Muted},
						}
						if s.config.CandidateSkin.AccentFill {
							fields = append(fields, skinColorField{"选中背景", d.Accent, &s.config.CandidateSkin.Accent})
						}
						fields = append(fields, skinColorField{"选中文字", d.AccentText, &s.config.CandidateSkin.AccentText})
						ui.Row(c).FillWidth().Wrap().GapX(10).GapY(10).Children(func() {
							for _, item := range fields {
								col := parseSkinColor(*item.field, item.fallback)
								ui.Row(c).AlignItems(ui.Center).Gap(10).Padding(6, 10).Radius(controlRadius).
									Background(p.card).Border(1, p.subBorder).MinWidth(max(176, textSize(c, 176))).MaxWidthPercent(100).Children(func() {
									ui.Column(c).Grow(1).MinWidth(0).Gap(1).Children(func() {
										ui.Text(c, item.label).FontSize(textSize(c, 13)).FontWeight(500).SingleLine()
										ui.Text(c, strings.ToUpper(formatHexRGB(col))).FontSize(textSize(c, 12)).TextColor(p.muted).FontFeatures("tnum").SingleLine()
									})
									if ui.ColorWell(c, &col).Label(item.label).Changed() {
										col.A = 255
										*item.field = formatHexRGB(col)
										colorChanged = true
									}
								})
							}
						})
						bg := parseSkinColor(s.config.CandidateSkin.Background, d.Background)
						textCol := parseSkinColor(s.config.CandidateSkin.Text, d.Text)
						accentCol := parseSkinColor(s.config.CandidateSkin.Accent, d.Accent)
						accentTextCol := parseSkinColor(s.config.CandidateSkin.AccentText, d.AccentText)
						selBg := bg
						if s.config.CandidateSkin.AccentFill {
							selBg = accentCol
						}
						if colorContrast(textCol, bg) < 3 || colorContrast(accentTextCol, selBg) < 3 {
							ui.Text(c, "当前文字与背景对比度较低，可能不易辨认").FontSize(textSize(c, 12.5)).TextColor(p.muted).Wrap()
						}
					})
				})
			})
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
			changed = choose(c, "行内预编辑", []choice{{"segmented", "拼音分词"}, {"spaces", "空格分音节"}, {"raw", "原始按键"}, {"off", "不显示"}}, &s.config.InlinePreedit, 160) || changed
		})
		changed = s.settingSwitch(c, "candidates/候选窗预编辑", false, &s.config.CandidatePreedit) || changed
		changed = s.settingSwitch(c, "candidates/整句编辑区", false, &s.config.SentenceEditor) || changed
		if s.config.SentenceEditor || s.showSetting("candidates/整句编辑区位置") {
			s.settingSubRow(c, "candidates/整句编辑区位置", func() {
				changed = choose(c, "整句编辑区位置", []choice{{"below", "候选窗下方"}, {"above", "候选窗上方"}}, &s.config.SentenceEditorPosition, 160) || changed
			})
		}
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
	} else if colorChanged {
		s.scheduleSaveConfig()
	}
}

// selectedLook 是候选窗里选中候选的样子。
type selectedLook struct {
	block bool     // 画底色块；搜狗样式不画
	fill  ui.Color // 底色块的颜色
	text  ui.Color // 选中候选的字（和序号）
}

type candidateTheme struct {
	background, border, text, muted ui.Color
	block                           bool
	accent, accentText              ui.Color
}

// 预设配色按浅色 / 深色成组排列，采用色相微偏中性底色与低眩光强调色，
// 保证正文、拼音序号与选中文字均满足高可读对比度。
var candidateSkinTemplates = []struct {
	name string
	skin CandidateSkin
}{
	// 浅色
	{"霜白", CandidateSkin{Background: "#fafbfc", Border: "#dcdfe6", Text: "#181c24", Muted: "#5e6675", Accent: "#e4eeff", AccentText: "#174ea6", AccentFill: true}},
	{"宣纸", CandidateSkin{Background: "#f9f6ef", Border: "#dfd7c7", Text: "#26211b", Muted: "#6e6456", Accent: "#a9442a", AccentText: "#fcfaf5", AccentFill: true}},
	{"青瓷", CandidateSkin{Background: "#f6f8f6", Border: "#d5dfd8", Text: "#1c2620", Muted: "#58695f", Accent: "#ddebe2", AccentText: "#165a3c", AccentFill: true}},
	{"暮樱", CandidateSkin{Background: "#fbf7f6", Border: "#e6d8d8", Text: "#2d2228", Muted: "#75616b", Accent: "#f4e1e4", AccentText: "#8f2d4c", AccentFill: true}},
	// 深色
	{"墨夜", CandidateSkin{Background: "#181a1f", Border: "#2d313b", Text: "#e6e9ef", Muted: "#8e96a6", Accent: "#26354d", AccentText: "#8ec0ff", AccentFill: true}},
	{"月影", CandidateSkin{Background: "#1e1e2e", Border: "#383a52", Text: "#cdd6f4", Muted: "#9399b2", Accent: "#b4befe", AccentText: "#181825", AccentFill: true}},
	{"极夜", CandidateSkin{Background: "#242933", Border: "#3b4252", Text: "#e5e9f0", Muted: "#9aa5b8", Accent: "#88c0d0", AccentText: "#1d222b", AccentFill: true}},
	{"松烟", CandidateSkin{Background: "#1d1b18", Border: "#36322b", Text: "#eae3d5", Muted: "#9d9485", Accent: "#3a3022", AccentText: "#e8b86d", AccentFill: true}},
}

func sameCandidateSkin(a, b CandidateSkin) bool {
	if a.AccentFill != b.AccentFill {
		return false
	}
	d := defaultCandidateSkin()
	norm := func(raw, fallback string) string {
		return formatHexRGB(parseSkinColor(raw, fallback))
	}
	return norm(a.Background, d.Background) == norm(b.Background, d.Background) &&
		norm(a.Border, d.Border) == norm(b.Border, d.Border) &&
		norm(a.Text, d.Text) == norm(b.Text, d.Text) &&
		norm(a.Muted, d.Muted) == norm(b.Muted, d.Muted) &&
		norm(a.Accent, d.Accent) == norm(b.Accent, d.Accent) &&
		norm(a.AccentText, d.AccentText) == norm(b.AccentText, d.AccentText)
}

func skinPresetChip(c *ui.Context, name string, skin CandidateSkin, active bool) bool {
	p := colors(c)
	d := defaultCandidateSkin()
	bg := parseSkinColor(skin.Background, d.Background)
	border := parseSkinColor(skin.Border, d.Border)
	textCol := parseSkinColor(skin.Text, d.Text)
	accent := parseSkinColor(skin.Accent, d.Accent)
	if !skin.AccentFill {
		accent = parseSkinColor(skin.AccentText, d.AccentText)
	}
	chip := ui.ButtonBase(c).MinHeight(max(32, textSize(c, 32))).Padding(5, 10).Gap(8).
		Radius(controlRadius).AlignItems(ui.Center).Cursor(ui.CursorPointer).Label(name)
	chipBorder := p.subBorder
	if active {
		chipBorder = c.Theme().Focus
		if c.Preferences().HighContrast {
			chipBorder = p.border
		}
		chip.Background(p.chosen).Border(1, chipBorder)
	} else {
		if chip.Hovered() {
			chip.Background(p.cardHover)
		} else {
			chip.Background(p.card)
		}
		chip.Border(1, chipBorder)
	}
	chip.Children(func() {
		ui.Row(c).Width(24).Height(16).Padding(2, 3).Gap(2).Radius(4).AlignItems(ui.Center).
			Background(bg).Border(1, border).Children(func() {
			ui.Row(c).Width(9).Height(8).Radius(2).Background(accent)
			ui.Row(c).Width(6).Height(3).Radius(1).Background(textCol)
		})
		label := ui.Text(c, name).FontSize(textSize(c, 13)).SingleLine()
		if active {
			label.FontWeight(600)
		}
	})
	return chip.Clicked()
}

func parseHexRGB(raw string) (ui.Color, bool) {
	hex := strings.TrimPrefix(strings.TrimSpace(raw), "#")
	switch len(hex) {
	case 3:
		v, err := strconv.ParseUint(hex, 16, 12)
		if err != nil {
			return ui.Color{}, false
		}
		return ui.RGB(uint8((v>>8)&0xf)*17, uint8((v>>4)&0xf)*17, uint8(v&0xf)*17), true
	case 6:
		v, err := strconv.ParseUint(hex, 16, 24)
		if err != nil {
			return ui.Color{}, false
		}
		return ui.RGB(uint8(v>>16), uint8((v>>8)&0xff), uint8(v&0xff)), true
	default:
		return ui.Color{}, false
	}
}

func parseSkinColor(hex, fallback string) ui.Color {
	if col, ok := parseHexRGB(hex); ok {
		return col
	}
	if col, ok := parseHexRGB(fallback); ok {
		return col
	}
	return ui.RGB(255, 255, 255)
}

func formatHexRGB(c ui.Color) string {
	return fmt.Sprintf("#%02x%02x%02x", c.R, c.G, c.B)
}

func colorLuminance(color ui.Color) float64 {
	linear := func(channel uint8) float64 {
		value := float64(channel) / 255
		if value <= 0.04045 {
			return value / 12.92
		}
		return math.Pow((value+0.055)/1.055, 2.4)
	}
	return 0.2126*linear(color.R) + 0.7152*linear(color.G) + 0.0722*linear(color.B)
}

func colorContrast(first, second ui.Color) float64 {
	firstLight, secondLight := colorLuminance(first), colorLuminance(second)
	return (math.Max(firstLight, secondLight) + 0.05) / (math.Min(firstLight, secondLight) + 0.05)
}

// candidateStyle 和候选窗（windows/tsf/src/ui.rs 的 theme_for_appearance）用一样的颜色，改一边要改另一边。
func candidateStyle(id string, dark bool) selectedLook {
	switch id {
	case "blue":
		if dark {
			return selectedLook{true, ui.Hex("#3d7ff0"), ui.Hex("#18181b")}
		}
		return selectedLook{true, ui.Hex("#2b6ce6"), ui.Hex("#ffffff")}
	case "green":
		// 微信绿保留绿底白字。
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

func resolveCandidateTheme(cfg Config, dark bool) candidateTheme {
	if cfg.CandidateStyle == "custom" {
		d := defaultCandidateSkin()
		return candidateTheme{
			background: parseSkinColor(cfg.CandidateSkin.Background, d.Background),
			border:     parseSkinColor(cfg.CandidateSkin.Border, d.Border),
			text:       parseSkinColor(cfg.CandidateSkin.Text, d.Text),
			muted:      parseSkinColor(cfg.CandidateSkin.Muted, d.Muted),
			block:      cfg.CandidateSkin.AccentFill,
			accent:     parseSkinColor(cfg.CandidateSkin.Accent, d.Accent),
			accentText: parseSkinColor(cfg.CandidateSkin.AccentText, d.AccentText),
		}
	}
	look := candidateStyle(cfg.CandidateStyle, dark)
	if dark {
		return candidateTheme{
			background: ui.Hex("#2b2b2b"),
			border:     ui.Hex("#484848"),
			text:       ui.Hex("#eeeeee"),
			muted:      ui.Hex("#9a9a9a"),
			block:      look.block,
			accent:     look.fill,
			accentText: look.text,
		}
	}
	return candidateTheme{
		background: ui.Hex("#ffffff"),
		border:     ui.Hex("#d6d6d6"),
		text:       ui.Hex("#1f1f1f"),
		muted:      ui.Hex("#616161"),
		block:      look.block,
		accent:     look.fill,
		accentText: look.text,
	}
}

// candidatePreview 按现在的字体、字号、排列和皮肤颜色画一个候选窗的样子。
func (s *settings) candidatePreview(c *ui.Context) {
	p := colors(c)
	t := c.Theme()
	ct := resolveCandidateTheme(s.config, t.Dark)
	popupBg, popupBorder, mutedText, normalText := ct.background, ct.border, ct.muted, ct.text
	if c.Preferences().HighContrast {
		popupBg, popupBorder, mutedText, normalText = p.card, p.border, p.muted, t.Text
	}
	family, size := s.config.FontFamily, textSize(c, float32(s.config.FontSize))
	if s.config.FontFallbacks != "" {
		family += "," + s.config.FontFallbacks
	}
	all := []string{"你好", "拟好", "你", "泥", "妮", "逆", "呢", "倪", "腻", "溺"}
	words := all[:min(max(s.config.PageSize, 1), len(all))]
	// 顶部留白放在有背景的内边距中，吸顶后不能让滚动内容透过外边距。
	ui.Column(c).FillWidth().Gap(8).Padding(4, 0, 8, 0).Background(p.page).Sticky().Top(0).Children(func() {
		ui.Row(c).FillWidth().AlignItems(ui.Center).Gap(8).Children(func() {
			ui.Text(c, "预览").FontSize(textSize(c, 13)).FontWeight(500).TextColor(p.muted).Grow(1)
			s.logo(c, 24)
			if ui.Button(c, "移动示例光标").Clicked() {
				s.previewCaretMoved = !s.previewCaretMoved
			}
		})
		// 外层画安静的预览舞台，横排按真实字号绘制；超出可用宽度时在预览内滚动，不压缩候选词。
		ui.Column(c).FillWidth().Padding(16, rowInset, 12, rowInset).Radius(panelRadius).
			Background(p.subtle).Border(1, p.border).Children(func() {
			ui.Row(c).FillWidth().Gap(4).Padding(0, 4, 8, 4).Children(func() {
				ui.Text(c, "输入位置").FontSize(textSize(c, 12)).TextColor(p.muted)
				if s.previewCaretMoved {
					ui.Text(c, "示例文字").Font(family).FontSize(size)
				}
				inline := "ni'hao"
				switch s.config.InlinePreedit {
				case "raw":
					inline = "nihao"
				case "spaces":
					inline = "ni hao"
				case "off":
					inline = ""
				}
				ui.Text(c, inline+"│").Font(family).FontSize(size).TextColor(normalText)
			})
			// 外层用可视宽度对齐；内层按候选宽度收起，放不下时才缩到可视宽度。
			ui.Row(c).FillWidth().Justify(ui.Center).Label("候选预览区域").Children(func() {
				if s.config.CandidateFollow && s.previewCaretMoved {
					ui.Column(c).Width(72).Shrink(0)
				}
				ui.ScrollHorizontal(c).MinWidth(0).Shrink(1).Padding(2, 2, 10, 2).Children(func() {
					ui.Column(c).Gap(6).MaxWidthPercent(100).Children(func() {
						if s.config.SentenceEditor && s.config.SentenceEditorPosition == "above" {
							s.sentencePreview(c, family, size, ct)
						}
						popup := ui.Column(c).Padding(8, 10).Radius(panelRadius).Background(popupBg).Border(1, popupBorder).
							Label("候选预览窗口").AlignSelf(ui.Start).Shrink(0).Gap(4)
						if s.config.Horizontal && !s.config.CandidateSingleLine {
							popup.MaxWidth(460).Shrink(1)
						}
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
								ui.Text(c, preedit).Font(family).FontSize(preeditSize).TextColor(mutedText).Padding(0, 4)
							}
							list := ui.Row(c).Gap(2)
							if !s.config.Horizontal {
								list = ui.Column(c).Gap(2)
							} else if !s.config.CandidateSingleLine {
								list.Wrap().MaxWidth(440)
							}
							list.Children(func() {
								defer func() {
									// 和真的候选窗一样，最后是翻页箭头
									ui.Row(c).Gap(6).Padding(3, 8).Children(func() {
										ui.Text(c, "‹").Font(family).FontSize(size).TextColor(mutedText)
										ui.Text(c, "›").Font(family).FontSize(size).TextColor(normalText)
									})
								}()
								for i, w := range words {
									item := ui.Row(c).Gap(4).Padding(4, 8).Radius(controlRadius).AlignItems(ui.Center).Shrink(0)
									label, text := mutedText, normalText
									if i == 0 {
										if ct.block {
											item.Background(ct.accent)
										}
										label, text = ct.accentText, ct.accentText
									}
									item.Children(func() {
										ui.Text(c, fmt.Sprint((i+1)%10)).Font(family).FontSize(size * 0.78).TextColor(label)
										ui.Text(c, w).Font(family).FontSize(size).TextColor(text)
										if i == 0 && s.config.PinBadge != "off" {
											badge := map[string]string{"paperclip": "📎", "pushpin": "📌", "dot": "•"}[s.config.PinBadge]
											ui.Text(c, badge).FontSize(size * 0.65).TextColor(label)
										}
									})
								}
							})
						})
						if s.config.SentenceEditor && s.config.SentenceEditorPosition != "above" {
							s.sentencePreview(c, family, size, ct)
						}
					})
				})
			})
		})
	})
}

// 与独立编辑区保持相同的内容层级：词块、原始字母、低强调提示；不画投影。
func (s *settings) sentencePreview(c *ui.Context, family string, size float32, ct candidateTheme) {
	ui.Column(c).AlignSelf(ui.Start).Padding(8).Gap(4).Radius(controlRadius).
		Background(ct.background).Border(1, ct.border).Label("整句编辑区预览").Children(func() {
		ui.Row(c).Gap(3).Children(func() {
			for _, word := range []string{"你", "好"} {
				ui.Text(c, word).Font(family).FontSize(max(12, size*0.9)).TextColor(ct.text).Padding(4, 7).Radius(4)
			}
		})
		ui.Text(c, "nihao").Font(family).FontSize(max(10, size*0.7)).TextColor(ct.muted)
		ui.Text(c, "点词块修改").Font(family).FontSize(min(max(size*0.65, 11), 14)).TextColor(ct.muted)
	})
}

func (s *settings) typingPage(c *ui.Context) {
	pageHeader(c, "拼音与按键", "")
	if !s.loaded {
		return
	}
	changed := false
	section(c, "拼音", func() {
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "typing/双拼", false, &s.config.Shuangpin) || changed
		}, s.config.Shuangpin || s.showSetting("typing/双拼方案") || s.showSetting("typing/双拼展示键位"), func() {
			s.settingSubRow(c, "typing/双拼方案", func() {
				changed = choose(c, "双拼方案", schemes, &s.config.ShuangpinScheme, 180) || changed
			})
			changed = s.settingSwitch(c, "typing/双拼展示键位", true, &s.config.ShuangpinShowKeys) || changed
		})
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "typing/模糊音", false, &s.config.Fuzzy) || changed
		}, s.config.Fuzzy || s.showSetting("typing/模糊音分组"), func() {
			s.settingBlock(c, "typing/模糊音分组", func() {
				subContent(c, func() {
					ui.Row(c).FillWidth().Wrap().GapX(16).GapY(12).Children(func() {
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
		})
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "typing/按错键纠错", false, &s.config.TypoCorrection) || changed
		}, s.config.TypoCorrection || s.showSetting("typing/纠错类型") || s.showSetting("typing/纠错标记"), func() {
			s.settingSubRow(c, "typing/纠错类型", func() {
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
			changed = s.settingSwitch(c, "typing/纠错标记", true, &s.config.TypoMark) || changed
		})
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
		showDigitHalf := s.showSetting("typing/数字标点保持半角")
		showDigitExcept := s.showSetting("typing/数字后例外保持全角")
		showDigitSymbols := s.showSetting("typing/数字后保持全角的标点")
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "typing/中文标点", false, &s.config.FullwidthPunct) || changed
		}, s.config.FullwidthPunct || showDigitHalf || showDigitExcept || showDigitSymbols, func() {
			ui.Column(c).FillWidth().Children(func() {
				changed = s.settingSwitch(c, "typing/数字标点保持半角", true, &s.config.DigitHalfwidthPunct) || changed
				if (s.config.FullwidthPunct && s.config.DigitHalfwidthPunct) || showDigitExcept || showDigitSymbols {
					nestedSubGroup(c, func() {
						ui.Column(c).FillWidth().Children(func() {
							changed = s.settingSwitch(c, "typing/数字后例外保持全角", true, &s.config.DigitPunctExceptions) || changed
							if (s.config.FullwidthPunct && s.config.DigitHalfwidthPunct && s.config.DigitPunctExceptions) || showDigitSymbols {
								nestedSubGroup(c, func() {
									s.settingBlock(c, "typing/数字后保持全角的标点", func() {
										subContent(c, func() {
											ui.Row(c).FillWidth().Wrap().GapX(16).GapY(12).Children(func() {
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
								})
							}
						})
					})
				}
			})
		})
		changed = s.settingSwitch(c, "typing/双引号改用直角引号", false, &s.config.CornerQuotes) || changed
		changed = s.settingSwitch(c, "typing/成对标点自动补全", false, &s.config.AutoPair) || changed
		changed = s.settingSwitch(c, "typing/小键盘的点输入「.」", false, &s.config.NumpadDot) || changed
	})
	section(c, "中英文混排", func() {
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "typing/中英文之间自动加空格", false, &s.config.Autospace) || changed
		}, s.config.Autospace || s.showSetting("typing/汉字和字母之间") || s.showSetting("typing/汉字和数字之间"), func() {
			changed = s.settingSwitch(c, "typing/汉字和字母之间", true, &s.config.AutospaceLetters) || changed
			changed = s.settingSwitch(c, "typing/汉字和数字之间", true, &s.config.AutospaceDigits) || changed
		})
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
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "features/记录剪贴板历史", false, &s.config.ClipboardHistory) || changed
		}, s.config.ClipboardHistory || s.showSetting("features/清空剪贴板历史"), func() {
			s.settingSubRow(c, "features/清空剪贴板历史", func() {
				if ui.Button(c, "清空").Clicked() {
					s.clearClipboard()
				}
			})
		})
		changed = s.settingSwitch(c, "features/推荐 emoji", false, &s.config.EmojiSuggest) || changed
	})
	section(c, "智能输入", func() {
		s.neuralModelRow(c)
		s.enhancementRow(c)
		changed = s.settingSwitch(c, "features/长词前缀补全", false, &s.config.LongWordCompletion) || changed
		changed = s.settingSwitch(c, "features/邮箱、网址补全", false, &s.config.EmailCompletion) || changed
		changed = s.settingSwitch(c, "features/日期时间", false, &s.config.DateTime) || changed
		changed = s.settingSwitch(c, "features/V 模式：计算与数字大写", false, &s.config.Calculator) || changed
		changed = s.settingSwitch(c, "features/快捷短语混入候选", false, &s.config.QuickPhrases) || changed
		changed = s.settingSwitch(c, "features/输出繁体", false, &s.config.Traditional) || changed
	})
	section(c, "联想", func() {
		settingGroup(c, func() {
			changed = s.settingSwitch(c, "features/上屏联想", false, &s.config.PredictNext) || changed
		}, s.config.PredictNext || s.showSetting("features/数字键选联想词") || s.showSetting("features/空格选联想词") || s.showSetting("features/左右方向键选联想词") || s.showSetting("features/双击空格键选词"), func() {
			changed = s.settingSwitch(c, "features/数字键选联想词", true, &s.config.PredictDigits) || changed
			ui.Column(c).FillWidth().Children(func() {
				changed = s.settingSwitch(c, "features/空格选联想词", true, &s.config.PredictSpace) || changed
				if s.config.PredictSpace || s.showSetting("features/双击空格键选词") {
					nestedSubGroup(c, func() {
						changed = s.settingSwitch(c, "features/双击空格键选词", true, &s.config.PredictDoubleSpace) || changed
					})
				}
			})
			changed = s.settingSwitch(c, "features/左右方向键选联想词", true, &s.config.PredictArrows) || changed
		})
	})
	if s.config.PredictNext {
		hint(c, "联想词来自内置语言模型和学习记录；常用搭配优先显示；搭配记录随学习备份导入导出")
	}
	s.appPresetSection(c)
	s.vimSection(c)
	section(c, "英文候选与拆字查字", func() {
		changed = s.settingSwitch(c, "features/英文拼写建议", false, &s.config.EnglishCorrection) || changed
		changed = s.settingSwitch(c, "features/英文上屏补空格", false, &s.config.EnglishSpace) || changed
		changed = s.settingSwitch(c, "features/英文音标与释义", false, &s.config.EnglishGloss) || changed
	})
	hint(c, "在输入位置打开工具栏，选择英文候选或拆字查字；也可在快捷键页启用专用按键；英文 Esc 返回，拆字选中后返回拼音")
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
		s.settingRow(c, "shortcuts/英文候选", func() {
			changed = choose(c, "英文候选", []choice{{"off", "关闭"}, {"ctrl+shift+y", "Ctrl + Shift + Y"}, {"ctrl+alt+y", "Ctrl + Alt + Y"}}, &s.config.HotkeyEnglish, 180) || changed
		})
		s.settingRow(c, "shortcuts/拆字查字", func() {
			changed = choose(c, "拆字查字", []choice{{"off", "关闭"}, {"ctrl+shift+u", "Ctrl + Shift + U"}, {"ctrl+alt+u", "Ctrl + Alt + U"}}, &s.config.HotkeyRadical, 180) || changed
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
	if s.status.LexiconWarning != "" {
		hint(c, s.status.LexiconWarning)
	}
	hint(c, "默认词库包含白霜、万象补充和项目热词；自定义主词库每行填写拼音、词语和词频，用制表符分隔；文件无法载入时使用默认词库；其他格式请使用下方的导入词库")
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
			ui.Column(c).FillWidth().Gap(1).Dividers(1, colors(c).border).Children(func() {
				for _, d := range s.dicts {
					name := d.Name
					detail := fmt.Sprintf("%d 条", d.Entries)
					if !d.Enabled {
						detail += " · 已停用"
					}
					if desc := strings.TrimSpace(d.Description); desc != "" {
						detail += " · " + desc
					}
					row(c, name, detail, func() {
						enabled := d.Enabled
						if checkbox(c, &enabled, "启用").Label("启用 " + name).Disabled(s.dictionaryBusy).Changed() {
							s.changeDictionary(d, d.Name, enabled, d.Description)
						}
						browseText := "浏览"
						if s.dictionaryName == name {
							browseText = "浏览中"
						}
						if ui.Button(c, browseText).Label("浏览 " + name).Disabled(s.dictionaryBusy).Clicked() {
							s.dictionaryName, s.dictionaryRename, s.dictionaryDescription = name, name, d.Description
							s.dictionaryQuery = ""
							s.queryDictionary(0)
						}
						if ui.Button(c, "删除").Label("删除 " + name).Disabled(s.dictionaryBusy).Clicked() {
							s.removeDictionary(name)
						}
					})
				}
			})
		})
	})
	s.dictionaryBrowser(c)
}

func (s *settings) phrasesPage(c *ui.Context) {
	if s.selectedPhrases == nil {
		s.selectedPhrases = map[string]bool{}
	}
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
			ui.Row(c).Gap(12).Children(func() {
				if ui.Button(c, "选择当前结果").Label("全选当前短语").Clicked() {
					for _, i := range shown {
						s.selectedPhrases[s.phrases[i].Code+"\t"+s.phrases[i].Text] = true
					}
				}
				if ui.Button(c, "删除选中").Label("删除选中短语").Disabled(s.phrasesBusy || len(s.selectedPhrases) == 0).Clicked() {
					s.confirmPhraseBatch = true
				}
			})
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
					selectedKey := ph.Code + "\t" + ph.Text
					checked := s.selectedPhrases[selectedKey]
					if checkbox(c, &checked, "").Label(fmt.Sprintf("选择短语 %s %d", ph.Code, i+1)).Disabled(s.phrasesBusy).Changed() {
						if checked {
							s.selectedPhrases[selectedKey] = true
						} else {
							delete(s.selectedPhrases, selectedKey)
						}
					}
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
	if ui.AlertDialog(c, &s.confirmPhraseBatch, "删除选中短语", "删除所选短语；可用备份恢复", "取消", "确认删除短语") == 1 {
		var kept []QuickPhrase
		for _, phrase := range s.phrases {
			if !s.selectedPhrases[phrase.Code+"\t"+phrase.Text] {
				kept = append(kept, phrase)
			}
		}
		s.savePhrases(kept, func() { s.selectedPhrases = map[string]bool{}; s.notice = "已删除所选短语" })
	}
}

func (s *settings) learnedPage(c *ui.Context) {
	if s.selectedWords == nil {
		s.selectedWords = map[string]bool{}
	}
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
	ui.Row(c).Gap(12).Children(func() {
		if ui.Button(c, "选择当前结果").Label("全选当前学习词").Clicked() {
			for _, word := range words {
				s.selectedWords[word.Word] = true
			}
		}
		if ui.Button(c, "删除选中").Label("删除选中学习词").Disabled(len(s.selectedWords) == 0).Clicked() {
			s.confirmWordBatch = true
		}
	})
	if ui.AlertDialog(c, &s.confirmWordBatch, "删除选中学习词", "删除所选词的学习记录和搭配；可用学习备份恢复", "取消", "确认删除学习词") == 1 {
		var selected []string
		for _, word := range s.words {
			if s.selectedWords[word.Word] {
				selected = append(selected, word.Word)
			}
		}
		s.forgetWords(selected)
		s.selectedWords = map[string]bool{}
	}
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
					checked := s.selectedWords[w.Word]
					if checkbox(c, &checked, "").Label("选择学习词 " + w.text()).Changed() {
						if checked {
							s.selectedWords[w.Word] = true
						} else {
							delete(s.selectedWords, w.Word)
						}
					}
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
