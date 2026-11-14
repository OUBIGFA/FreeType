package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/egoist/mygo/ui"
)

// 页面：ID、侧边栏名字、图标。
var pages = []struct {
	ID, Name string
	Icon     **ui.SVG
}{
	{"general", "常规", &icons.general},
	{"candidates", "候选窗", &icons.candidates},
	{"typing", "拼音与按键", &icons.typing},
	{"features", "实用功能", &icons.features},
	{"shortcuts", "快捷键", &icons.shortcuts},
	{"dicts", "词库", &icons.dicts},
	{"phrases", "快捷短语", &icons.phrases},
	{"common", "常用语", &icons.common},
	{"learned", "学到的词", &icons.learned},
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
		ui.Column(c).Width(236).FillHeight().Padding(4, 10, 12, 10).Gap(14).Children(func() {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Padding(0, 8).Children(func() {
				s.logo(c, 30)
				ui.Column(c).Children(func() {
					ui.Text(c, "FreeType").FontSize(15).FontWeight(600)
					ui.Text(c, "输入法设置").FontSize(12).TextColor(p.muted)
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
		ui.Scroll(c).Key(contentKey).TrackScroll(scroll).Grow(1).FillHeight().Padding(6, 32, 40, 32).Children(func() {
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
				case "about":
					s.aboutPage(c)
				default:
					s.generalPage(c)
				}
			})
		})
	})
}

// logo 画安装目录里的图标（测试里没有，就画一个字母）。
func (s *settings) logo(c *ui.Context, size float32) {
	if s.resources != "" {
		if data, err := os.ReadFile(filepath.Join(s.resources, "icons", "logo.png")); err == nil {
			if img, err := ui.DecodeImage(data); err == nil {
				ui.Image(c, img).Size(size, size)
				return
			}
		}
	}
	ui.Text(c, "F").FontSize(size*0.6).FontWeight(700).TextColor(c.Theme().AccentText).
		Size(size, size).Radius(size / 4).Background(c.Theme().Accent).Center()
}

// statusCard 是常规页顶上的服务状态。
func (s *settings) statusCard(c *ui.Context) {
	p := colors(c)
	t := c.Theme()
	ui.Row(c).Gap(16).Padding(18, 20).Radius(8).Background(p.card).Border(1, p.border).AlignItems(ui.Center).Children(func() {
		s.logo(c, 44)
		ui.Column(c).Grow(1).Gap(4).Children(func() {
			ui.Text(c, "FreeType 输入法").FontSize(17).FontWeight(600)
			switch {
			case s.statusErr != "":
				ui.Text(c, "无法连接输入法服务："+s.statusErr).TextColor(t.Danger).Wrap()
			case s.status.Error != "":
				ui.Text(c, "词库载入失败："+s.status.Error).TextColor(t.Danger).Wrap()
			case !s.status.Ready:
				ui.Text(c, "输入法服务正在载入词库…").TextColor(p.muted)
			default:
				ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
					ui.Text(c, "●").FontSize(10).TextColor(t.Success)
					ui.Text(c, "正在运行 · 版本 "+s.version).TextColor(p.muted)
				})
			}
		})
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
		changed = s.settingSwitch(c, "general/切换状态时提示", false, &s.config.HintOnToggle) || changed
		changed = s.settingSwitch(c, "general/切换输入框时提示", false, &s.config.HintOnFocus) || changed
		s.settingRow(c, "general/提示位置", func() {
			changed = choose(c, "提示位置", positions, &s.config.HintPosition, 200) || changed
		})
	})
	s.backupSection(c)
	if changed {
		s.saveConfig()
	}
}

func (s *settings) candidatesPage(c *ui.Context) {
	pageHeader(c, "候选窗", "打字时光标下方显示候选词的小窗口")
	s.candidatePreview(c)
	if !s.loaded {
		return
	}
	changed := false
	section(c, "外观", func() {
		s.settingRow(c, "candidates/字体", func() {
			family := s.config.FontFamily
			ui.Column(c).Children(func() {
				if ui.Combobox(c, &family, s.fontList(fontNames)).Label("字体").Width(240).Changed() && family != "" {
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
			return selectedLook{true, ui.Hex("#3d7ff0"), ui.Hex("#ffffff")}
		}
		return selectedLook{true, ui.Hex("#2b6ce6"), ui.Hex("#ffffff")}
	case "green":
		return selectedLook{true, ui.Hex("#07c160"), ui.Hex("#ffffff")}
	case "sogou":
		if dark {
			return selectedLook{false, ui.Color{}, ui.Hex("#6c9eff")}
		}
		return selectedLook{false, ui.Color{}, ui.Hex("#1e6fff")}
	default:
		return selectedLook{true, brandPink, ui.Hex("#18181b")}
	}
}

// candidatePreview 按现在的字体、字号、排列和强调色画一个候选窗的样子。
func (s *settings) candidatePreview(c *ui.Context) {
	p := colors(c)
	t := c.Theme()
	family, size := s.config.FontFamily, float32(s.config.FontSize)
	all := []string{"你好", "拟好", "你", "泥", "妮", "逆", "呢", "倪", "腻", "溺"}
	words := all[:min(max(s.config.PageSize, 1), len(all))]
	ui.Column(c).Padding(24, 28).Radius(8).Background(p.card).Border(1, p.border).AlignItems(ui.Center).Gap(10).Children(func() {
		ui.Text(c, "预览").FontSize(12).TextColor(p.muted)
		box := ui.Column(c).Padding(8).Radius(8).Background(t.Background).Border(1, p.border).
			Shadow(0, 4, 14, 0, ui.RGBA(0, 0, 0, 0.12)).Gap(4)
		box.Children(func() {
			ui.Text(c, "ni hao").Font(family).FontSize(size*0.78).TextColor(p.muted).Padding(0, 4)
			list := ui.Row(c).Gap(2)
			if !s.config.Horizontal {
				list = ui.Column(c).Gap(2)
			}
			list.Children(func() {
				defer func() {
					// 和真的候选窗一样，最后是翻页箭头
					ui.Row(c).Gap(6).Padding(3, 8).Children(func() {
						ui.Text(c, "‹").Font(family).FontSize(size).TextColor(p.border)
						ui.Text(c, "›").Font(family).FontSize(size).TextColor(t.Text)
					})
				}()
				style := candidateStyle(s.config.CandidateStyle, t.Dark)
				for i, w := range words {
					item := ui.Row(c).Gap(4).Padding(3, 6).Radius(6).AlignItems(ui.Center)
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
}

func (s *settings) typingPage(c *ui.Context) {
	pageHeader(c, "拼音与按键", "")
	if !s.loaded {
		return
	}
	changed := false
	section(c, "拼音", func() {
		changed = s.settingSwitch(c, "typing/双拼", false, &s.config.Shuangpin) || changed
		if s.config.Shuangpin || s.showSetting("typing/双拼方案") {
			s.settingSubRow(c, "typing/双拼方案", func() {
				changed = choose(c, "双拼方案", schemes, &s.config.ShuangpinScheme, 180) || changed
			})
		}
		changed = s.settingSwitch(c, "typing/模糊音", false, &s.config.Fuzzy) || changed
		if s.config.Fuzzy || s.showSetting("typing/模糊音分组") {
			s.settingBlock(c, "typing/模糊音分组", func() {
				ui.Row(c).Wrap().GapX(18).GapY(8).Padding(12, 18, 14, 46).Background(colors(c).subtle).Children(func() {
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
		}
		changed = s.settingSwitch(c, "typing/按错键纠错", false, &s.config.TypoCorrection) || changed
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
		changed = s.settingSwitch(c, "typing/以词定字", false, &s.config.PickChar) || changed
	})
	section(c, "标点", func() {
		changed = s.settingSwitch(c, "typing/中文标点", false, &s.config.FullwidthPunct) || changed
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
		changed = s.settingSwitch(c, "features/邮箱、网址补全", false, &s.config.EmailCompletion) || changed
		changed = s.settingSwitch(c, "features/日期时间", false, &s.config.DateTime) || changed
		changed = s.settingSwitch(c, "features/V 模式：计算与数字大写", false, &s.config.Calculator) || changed
		changed = s.settingSwitch(c, "features/快捷短语混入候选", false, &s.config.QuickPhrases) || changed
		changed = s.settingSwitch(c, "features/输出繁体", false, &s.config.Traditional) || changed
	})
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
	hint(c, "快捷键默认关闭。启用后，FreeType 会先接收该按键；应用中的同名快捷键将无法使用。")
	section(c, "固定按键", func() {
		s.settingBlock(c, "shortcuts/固定按键", func() {
			for _, k := range [][2]string{
				{"Shift", "切换中文 / 英文（可在「常规」里改）"},
				{"空格 / 1–9 / 0", "空格选择当前候选；数字选择对应候选，每页有 10 项时，0 选择第 10 项；回车上屏原始输入"},
				{"- =  PgUp PgDn", "翻页；其他翻页键可在「拼音与按键」中设置"},
				{"← →  ↓", "移动候选；横排时 ↓ 展开更多候选"},
				{"Ctrl + Delete", "删除光标处学错的词的记忆（也可以长按或右键候选）"},
				{"[ ]", "启用「以词定字」后，选择候选的第一个 / 最后一个字；设置为翻页键时用于翻页"},
				{"v + 空格", "启用工具栏后，在全拼状态下打开剪贴板、常用语、符号和表情"},
				{"Esc", "清空当前输入，或关闭工具栏和表情面板"},
			} {
				ui.Row(c).Padding(11, 18).Gap(16).AlignItems(ui.Center).Children(func() {
					ui.Row(c).Width(180).Gap(4).Children(func() {
						for _, part := range strings.Split(k[0], "  ") {
							keyCap(c, part)
						}
					})
					ui.Text(c, k[1]).Grow(1).Wrap()
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
			ui.Text(c, e).FontSize(12.5).TextColor(c.Theme().Danger).Wrap()
		}
	})
}

func (s *settings) dictsPage(c *ui.Context) {
	pageHeader(c, "词库", "导入的词参与组词和排序，不会混进你学到的词")
	section(c, "主词库", func() {
		current := "白霜＋万象＋项目热词（默认）"
		if s.config.Lexicon != "" {
			current = s.config.Lexicon
		}
		s.settingRowValue(c, "dicts/词库文件", current, func() {
			if ui.Button(c, "选择文件…").Label("选择主词库文件").Clicked() && s.openTable != nil {
				s.background(func() func() {
					path := s.openTable("选择主词库（lexicon.tsv）")
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
	hint(c, "默认使用白霜主词库、万象补充和项目热词。自定义主词库需使用 TSV 文件：每行包含拼音、词语和词频，列之间用制表符分隔。Rime 等词库可从下方「导入词库」导入。选择文件后，FreeType 会在后台载入词库；文件不存在时使用默认词库。")
	section(c, "导入词库", func() {
		s.settingRow(c, "dicts/导入词库", func() {
			if ui.PrimaryButton(c, "导入词库…").Clicked() {
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
	pageHeader(c, "快捷短语", "启用「快捷短语候选」后，输入完整编码即可选用短语。编码只使用英文字母。")
	section(c, "添加", func() {
		s.settingBlock(c, "phrases/添加快捷短语", func() {
			ui.Column(c).Padding(16, 18).Gap(12).Children(func() {
				ui.Column(c).Children(func() {
					ui.Form(c, func() {
						ui.Field(c, "编码", func() { ui.TextInput(c, &s.newCode).Placeholder("例如 dz").Width(220) })
						ui.Field(c, "短语", func() { ui.TextArea(c, &s.newText).Height(72) }).Error(s.formError)
						ui.Field(c, "权重", func() { ui.TextInput(c, &s.newWeight).Placeholder("10").Width(100) }).
							Description("同一编码有多条时，权重大的在前。")
					})
				})
				ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
					if ui.PrimaryButton(c, "添加").Clicked() {
						s.addPhrase()
					}
				})
			})
		})
	})
	section(c, "批量导入、导出", func() {
		s.settingRow(c, "phrases/导入或导出", func() {
			checkbox(c, &s.replacePhrases, "替换现有的")
			if ui.Button(c, "导入…").Clicked() {
				s.importPhrases()
			}
			if ui.Button(c, "导出…").Clicked() {
				s.export("export_quick_phrases", "导出快捷短语", "FreeType 快捷短语.txt")
			}
		})
	})
	s.reportView(c)
	ui.Row(c).Margin(16, 0, 0, 0).Gap(12).AlignItems(ui.Center).Children(func() {
		ui.Text(c, fmt.Sprintf("全部快捷短语（%d）", len(s.phrases))).FontSize(14).FontWeight(600).Grow(1)
		ui.SearchField(c, &s.phraseQuery).Placeholder("搜索编码或短语").Width(240)
	})
	section(c, "", func() {
		s.settingBlock(c, "phrases/全部快捷短语", func() {
			shown := s.filteredPhrases()
			if len(shown) == 0 {
				title := "没有快捷短语"
				if strings.TrimSpace(s.phraseQuery) != "" {
					title = "没有匹配的快捷短语"
				}
				row(c, title, "", nil)
				return
			}
			remove := -1
			// 最多显示 12 行，多出来的在列表里滚动
			ui.List(c, nil, len(shown), func(k int) {
				i := shown[k]
				ph := s.phrases[i]
				ui.Row(c).Height(phraseRow).Padding(0, 18).Gap(14).AlignItems(ui.Center).BorderWidth(0, 0, 1, 0).BorderColor(colors(c).border).Children(func() {
					keyCap(c, ph.Code)
					ui.Text(c, preview(ph.Text)).Grow(1).Shrink(1).SingleLine()
					ui.Textf(c, "%d", ph.Weight).TextColor(colors(c).muted).FontSize(12.5)
					if ui.Button(c, "删除").Label(fmt.Sprintf("删除 %s %d", ph.Code, i+1)).Clicked() {
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
	pageHeader(c, "学到的词", "查看和管理自动学习的词。选词次数和上下文搭配会影响候选排序；可在这里删除错误记录，也可长按候选或按 Ctrl+Delete 删除。")
	section(c, "", func() {
		s.settingRow(c, "learned/学习备份", func() {
			if ui.Button(c, "导入…").Clicked() {
				s.importLearning()
			}
			if ui.Button(c, "导出…").Clicked() {
				s.export("export_learning", "导出学习备份", "FreeType 学习备份.json")
			}
		})
		s.settingRow(c, "learned/全部清空", func() {
			if ui.Button(c, "全部清空…").Clicked() {
				s.confirmClear = true
			}
		})
	})
	section(c, "添加词", func() {
		s.settingBlock(c, "learned/添加词", func() {
			ui.Column(c).Padding(16, 18).Gap(10).Children(func() {
				ui.Text(c, "中文词的拼音可留空，FreeType 会自动注音；多音字请填写拼音。英文词默认使用原词的小写作为输入码。中英混输词请填写实际输入码，如 datag → 打 tag。").FontSize(12.5).TextColor(colors(c).muted).Wrap()
				ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
					ui.Column(c).Children(func() {
						ui.TextInput(c, &s.newWord).Placeholder("词语，例如 霜频阁").Label("词语").Width(200)
					})
					ui.Column(c).Grow(1).Children(func() {
						ui.TextInput(c, &s.newPinyin).Placeholder("如 shuang pin ge 或 datag").Label("拼音或输入码").FillWidth()
					})
					if ui.PrimaryButton(c, "添加").Label("添加词").Clicked() {
						s.addWord()
					}
				})
				if s.wordError != "" {
					ui.Text(c, s.wordError).FontSize(12.5).TextColor(c.Theme().Danger).Wrap()
				}
			})
		})
	})
	switch ui.AlertDialog(c, &s.confirmClear, "清空学习记录？", "将删除全部学习词条、选词次数和搭配。此操作需要学习备份才能恢复。", "取消", "清空") {
	case 1:
		s.clearLearned()
	}
	words := s.filteredWords()
	ui.Row(c).Margin(16, 0, 0, 0).Gap(12).AlignItems(ui.Center).Children(func() {
		ui.Text(c, fmt.Sprintf("学到的词（%d）", len(s.words))).FontSize(14).FontWeight(600).Grow(1)
		if ui.Button(c, "刷新").Clicked() {
			s.loadWords()
		}
		ui.SearchField(c, &s.wordQuery).Placeholder("搜索词、拼音或输入码").Width(240)
	})
	p := colors(c)
	ui.Column(c).Margin(8, 0, 0, 0).Radius(8).Background(p.card).Border(1, p.border).Clip().Children(func() {
		s.settingBlock(c, "learned/词条列表", func() {
			if !s.wordsLoaded {
				row(c, "正在读取…", "", nil)
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
				ui.Row(c).Padding(9, 18).Gap(14).AlignItems(ui.Center).BorderWidth(0, 0, 1, 0).BorderColor(p.border).Children(func() {
					ui.Text(c, w.text()).Width(160).SingleLine()
					ui.Text(c, w.Pinyin).Grow(1).SingleLine().TextColor(p.muted)
					ui.Textf(c, "%.0f", w.Freq).FontSize(12.5).TextColor(p.muted).Width(70)
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
	phraseRow     = 46
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
