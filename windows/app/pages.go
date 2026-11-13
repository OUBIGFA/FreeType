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
	{"learned", "学到的词", &icons.learned},
	{"about", "关于", &icons.about},
}

func (s *settings) view(c *ui.Context) {
	if s.notice != "" {
		c.Toast(s.notice)
		s.notice = ""
	}
	c.SetTheme(branded(c))
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
			ui.Column(c).Gap(2).Grow(1).Children(func() {
				for _, pg := range pages {
					if navItem(c, *pg.Icon, pg.Name, s.page == pg.ID) && s.page != pg.ID {
						s.page = pg.ID
						if pg.ID == "learned" && !s.wordsLoaded {
							s.loadWords()
						}
					}
				}
			})
		})
		ui.Scroll(c).Grow(1).FillHeight().Padding(6, 32, 40, 32).Children(func() {
			// 宽度不变，窗口宽了左右留出一样的空
			ui.Column(c).MaxWidth(860).FillWidth().Margin(0, ui.Auto).Gap(6).Children(func() {
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
				ui.Text(c, "输入法服务没有运行："+s.statusErr).TextColor(t.Danger).Wrap()
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
		row(c, "中文 / 英文", "单按 Shift 也能切换", func() {
			if ui.Segmented(c, &mode, "中文", "英文").Label("中文 / 英文").Changed() {
				s.run(op("toggle_mode"))
			}
		})
		punct := 0
		if !s.config.FullwidthPunct {
			punct = 1
		}
		row(c, "标点", "中文状态下输入的标点", func() {
			if ui.Segmented(c, &punct, "中文标点", "英文标点").Label("标点").Changed() {
				s.run(command("toggle_punct"))
			}
		})
		script := 0
		if s.config.Traditional {
			script = 1
		}
		row(c, "简体 / 繁体", "输出的字形", func() {
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
		row(c, "切换键", "单按一下 Shift 在中文和英文之间切换", func() {
			changed = choose(c, "切换键", shiftToggles, &s.config.ShiftToggle, 200) || changed
		})
		row(c, "默认状态", "选「全局」时，所有应用沿用最后主动切换的中英文状态，重启后也保留", func() {
			changed = choose(c, "默认状态", modes, &s.config.DefaultMode, 200) || changed
		})
		if s.config.DefaultMode != "global" {
			row(c, "中英文状态", "选「每个应用分别记住」时，每个程序保留各自上次的中英文状态", func() {
				changed = choose(c, "中英文状态", modeScopes, &s.config.ModeScope, 200) || changed
			})
		}
	})
	section(c, "状态提示", func() {
		changed = switchRow(c, "显示托盘图标", "在任务栏通知区域显示 FreeType 图标，右键可切换状态、打开设置；关掉后可从开始菜单打开设置", &s.config.TrayIcon) || changed
		changed = switchRow(c, "悬浮状态栏", "在桌面上显示中 / 英、标点、简繁按钮，可拖动；右键可以隐藏", &s.config.StatusBar) || changed
		if s.config.StatusBar {
			changed = switchLine(c, "自动隐藏", "一会儿不用就收起，状态改变时再出现", &s.config.StatusBarAutohide, true) || changed
		}
		changed = switchRow(c, "切换状态时提示", "切换中英文、标点、简繁时，在光标旁短暂显示新的状态", &s.config.HintOnToggle) || changed
		changed = switchRow(c, "切换输入框时提示", "光标移到另一个输入框（同一窗口或别的窗口）时，提示当前的中英文状态", &s.config.HintOnFocus) || changed
		row(c, "提示位置", "提示显示在光标的哪一边", func() {
			changed = choose(c, "提示位置", positions, &s.config.HintPosition, 200) || changed
		})
	})
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
		row(c, "字体", "候选窗、提示使用的字体，默认是 Microsoft YaHei（微软雅黑）", func() {
			family := s.config.FontFamily
			grayed(c).Children(func() {
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
		row(c, "字号", "按 100% 缩放计的像素，12 到 36", func() {
			size := float64(s.config.FontSize)
			grayed(c).Children(func() {
				if ui.NumberInput(c, &size, 12, 36, 1).Label("字号").Width(120).Changed() {
					s.config.FontSize = int(size)
					changed = true
				}
			})
		})
		row(c, "候选强调色", "选中的候选怎么标出来；搜狗样式不画底色块，只把字换成蓝色", func() {
			changed = choose(c, "候选强调色", candidateStyleChoices, &s.config.CandidateStyle, 220) || changed
		})
		row(c, "每页候选个数", "", func() {
			size := fmt.Sprint(s.config.PageSize)
			if ui.Select(c, &size, pageSizes).Label("每页候选个数").Width(120).Changed() {
				fmt.Sscan(size, &s.config.PageSize)
				changed = true
			}
		})
		row(c, "排列方向", "横排时 ← → 选词、↓ 展开更多候选；竖排时 ↑ ↓ 选词", func() {
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
		changed = switchRow(c, "双拼", "用双拼方案打字；编辑区照样显示全拼", &s.config.Shuangpin) || changed
		if s.config.Shuangpin {
			subRow(c, "双拼方案", "", func() {
				changed = choose(c, "双拼方案", schemes, &s.config.ShuangpinScheme, 180) || changed
			})
		}
		changed = switchRow(c, "模糊音", "读音相近的拼音互相能打出来，如 zi 也出「知」", &s.config.Fuzzy) || changed
		if s.config.Fuzzy {
			ui.Row(c).Wrap().GapX(18).GapY(8).Padding(12, 18, 14, 46).Background(colors(c).subtle).Children(func() {
				for _, rule := range fuzzyRules {
					on := s.config.FuzzyGroups&rule.Bit != 0
					if ui.Checkbox(c, &on, rule.Label).Width(120).Changed() {
						if on {
							s.config.FuzzyGroups |= rule.Bit
						} else {
							s.config.FuzzyGroups &^= rule.Bit
						}
						changed = true
					}
				}
			})
		}
		changed = switchRow(c, "按错键纠错", "两个键按反了（weixni → 微信）、按到旁边的键也能出候选", &s.config.TypoCorrection) || changed
		changed = switchRow(c, "显示更多生僻字", "也给出 CJK 扩展 B 区以后的单字；常见字体显示不了这些字，会是方块", &s.config.RareChars) || changed
	})
	section(c, "选词与翻页", func() {
		row(c, "翻页键", "PgUp、PgDn 和方向键一直能用", func() {
			for _, k := range []struct {
				bit   int
				label string
			}{{pageKeyMinus, "- ="}, {pageKeyBracket, "[ ]"}, {pageKeyComma, ", ."}, {pageKeyTab, "Tab"}} {
				on := s.config.PageKeys&k.bit != 0
				if ui.Checkbox(c, &on, k.label).Changed() {
					if on {
						s.config.PageKeys |= k.bit
					} else {
						s.config.PageKeys &^= k.bit
					}
					changed = true
				}
			}
		})
		row(c, "分号、引号选词", "", func() {
			changed = choose(c, "分号、引号选词", selectKeys, &s.config.SelectKeys, 220) || changed
		})
		changed = switchRow(c, "以词定字", "打拼音时按 [ 上屏候选的第一个字，按 ] 上屏最后一个字（翻页键用了 [ ] 时让给翻页）", &s.config.PickChar) || changed
	})
	section(c, "标点", func() {
		changed = switchRow(c, "中文标点", "中文状态下输入 ，。？ 这类全角标点；数字后面的标点保持半角（3.14）", &s.config.FullwidthPunct) || changed
		changed = switchRow(c, "成对标点自动补全", "输入 （ “ 《 【 「 时自动补上另一半，光标放在中间", &s.config.AutoPair) || changed
		changed = switchRow(c, "小键盘的点输入「.」", "小键盘的点总是半角；打字途中按它进入网址、邮箱输入", &s.config.NumpadDot) || changed
	})
	section(c, "中英文混排", func() {
		changed = switchRow(c, "中英文之间自动加空格", "自由输入法FreeType → 自由输入法 FreeType；终端等读不到光标前文字的应用里不生效", &s.config.Autospace) || changed
		if s.config.Autospace {
			changed = switchLine(c, "汉字和字母之间", "", &s.config.AutospaceLetters, true) || changed
			changed = switchLine(c, "汉字和数字之间", "", &s.config.AutospaceDigits, true) || changed
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
		changed = switchRow(c, "v + 空格打开工具栏", "全拼下只打 v 再按空格：剪贴板、常用语、符号、表情", &s.config.Toolbar) || changed
		changed = switchRow(c, "记录剪贴板历史", "只在内存里记最近 20 条，不写到磁盘；密码管理器标明不记录的内容不记", &s.config.ClipboardHistory) || changed
		if s.config.ClipboardHistory {
			subRow(c, "清空剪贴板历史", "", func() {
				if ui.Button(c, "清空").Clicked() {
					s.clearClipboard()
				}
			})
		}
		changed = switchRow(c, "推荐 emoji", "打「开心」时候选里出现 😊，打「八」时出现 8️⃣", &s.config.EmojiSuggest) || changed
	})
	section(c, "智能输入", func() {
		changed = switchRow(c, "邮箱、网址补全", "输入 @ 显示 @qq.com、@gmail.com 等后缀；输入 gmail. 或 www.xxx. 显示 .com 等结尾", &s.config.EmailCompletion) || changed
		changed = switchRow(c, "日期时间", "输入 riqi、shijian、xingqi（或 rq、sj）得到现在的日期、时间、星期", &s.config.DateTime) || changed
		changed = switchRow(c, "V 模式：计算与数字大写", "v1+2*3 得到 7；v1314 得到 壹仟叁佰壹拾肆元整 和 一千三百一十四", &s.config.Calculator) || changed
		changed = switchRow(c, "快捷短语混入候选", "编码正好打完时，快捷短语排在第一个", &s.config.QuickPhrases) || changed
		changed = switchRow(c, "输出繁体", "候选和上屏的字都换成繁体", &s.config.Traditional) || changed
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
		row(c, "表情面板", "打拼音或英文搜索 emoji 和符号", func() {
			changed = choose(c, "表情面板", hotkeysEmoji, &s.config.HotkeyEmoji, 180) || changed
		})
		row(c, "中文 / 英文标点", "", func() {
			changed = choose(c, "中文 / 英文标点", hotkeysPunct, &s.config.HotkeyPunct, 180) || changed
		})
		row(c, "简体 / 繁体", "", func() {
			changed = choose(c, "简体 / 繁体", hotkeysTraditional, &s.config.HotkeyTraditional, 180) || changed
		})
		row(c, "切换中文 / 英文", "Shift 之外再加一个", func() {
			changed = choose(c, "切换中文 / 英文", hotkeysMode, &s.config.HotkeyMode, 180) || changed
		})
	})
	hint(c, "设了的快捷键由输入法先接收，应用里同样的快捷键（如编辑器的 Ctrl+. 快速修复）就不起作用了，所以默认都不用。")
	section(c, "固定按键", func() {
		for _, k := range [][2]string{
			{"Shift", "切换中文 / 英文（可在「常规」里改）"},
			{"空格 / 1–9", "选词；回车原样上屏打的字母"},
			{"- =  PgUp PgDn", "翻页"},
			{"← →  ↓", "移动候选；横排时 ↓ 展开更多候选"},
			{"Ctrl + Delete", "删除光标处学错的词的记忆（也可以长按或右键候选）"},
			{"[ ]", "以词定字：上屏第一个 / 最后一个字"},
			{"v + 空格", "工具栏：剪贴板、常用语、符号、表情"},
			{"Esc", "清空正在打的字"},
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
		current := "雾凇拼音（随安装包，默认）"
		if s.config.Lexicon != "" {
			current = s.config.Lexicon
		}
		row(c, "词库文件", current, func() {
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
	hint(c, "主词库是「拼音<Tab>词<Tab>词频」一行一条的 lexicon.tsv。万象拼音等 Rime 词库可以用 tools/rime2tsv.py、scripts/make-wanxiang.sh 转出来；换词库要几秒，学到的词不受影响。文件不在时用默认词库。")
	section(c, "导入词库", func() {
		row(c, "导入词库", "自动识别：搜狗细胞词库（.scel）、搜狗文本词库（'ni'hao 你好）、水杉、Rime、WowType 词库；只有词语的词表（如微信输入法导出的）按常用读音自动注音", func() {
			if ui.PrimaryButton(c, "导入词库…").Clicked() {
				s.importDictionary()
			}
		})
	})
	s.reportView(c)
	section(c, "已导入", func() {
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
}

func (s *settings) phrasesPage(c *ui.Context) {
	pageHeader(c, "快捷短语", "编码（只能是英文字母）正好打完时，短语排在候选第一个；也能在 v + 空格的「常用语」里选")
	section(c, "添加", func() {
		ui.Column(c).Padding(16, 18).Gap(12).Children(func() {
			// 表单里只有输入框，整个用深灰强调色
			grayed(c).Children(func() {
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
	section(c, "批量导入、导出", func() {
		row(c, "导入或导出", "导入自动识别：水杉（编码<Tab>短语）、Rime（短语<Tab>编码）、搜狗自定义短语和微信输入法常用语导出（编码,位置=短语）。导出为水杉格式", func() {
			ui.Checkbox(c, &s.replacePhrases, "替换现有的")
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
		ui.Text(c, fmt.Sprintf("全部短语（%d）", len(s.phrases))).FontSize(14).FontWeight(600).Grow(1)
		ui.SearchField(c, &s.phraseQuery).Placeholder("搜索编码或短语").Width(240)
	})
	section(c, "", func() {
		shown := s.filteredPhrases()
		if len(shown) == 0 {
			row(c, "没有快捷短语", "", nil)
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
}

func (s *settings) learnedPage(c *ui.Context) {
	pageHeader(c, "学到的词", "你选过的词组会被记住，下次排得更靠前。打错字被记住了，可以在这里删掉；打字时长按候选或按 Ctrl+Delete 也能删")
	section(c, "", func() {
		row(c, "学习备份", "保存词条、选词次数和搭配；导入时保留现有记录，重复导入不累加", func() {
			if ui.Button(c, "导入…").Clicked() {
				s.importLearning()
			}
			if ui.Button(c, "导出…").Clicked() {
				s.export("export_learning", "导出学习备份", "FreeType 学习备份.json")
			}
		})
		row(c, "全部清空", "删掉学到的全部词和输入习惯，候选回到刚安装时的顺序", func() {
			if ui.Button(c, "全部清空…").Clicked() {
				s.confirmClear = true
			}
		})
	})
	section(c, "添加词", func() {
		ui.Column(c).Padding(16, 18).Gap(10).Children(func() {
			ui.Text(c, "加进去的词马上能打出来，排在前面。拼音可以不填，按常用读音自动注音（多音字请自己填）").FontSize(12.5).TextColor(colors(c).muted).Wrap()
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				grayed(c).Children(func() {
					ui.TextInput(c, &s.newWord).Placeholder("词语，例如 霜频阁").Label("词语").Width(200)
				})
				grayed(c).Grow(1).Children(func() {
					ui.TextInput(c, &s.newPinyin).Placeholder("拼音，例如 shuang pin ge").Label("拼音").FillWidth()
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
	switch ui.AlertDialog(c, &s.confirmClear, "清空学到的词？", "学到的词和输入习惯都会删掉。没有备份时无法恢复。快捷短语和导入的词库不受影响。", "取消", "清空") {
	case 1:
		s.clearLearned()
	}
	words := s.filteredWords()
	ui.Row(c).Margin(16, 0, 0, 0).Gap(12).AlignItems(ui.Center).Children(func() {
		ui.Text(c, fmt.Sprintf("学到的词（%d）", len(s.words))).FontSize(14).FontWeight(600).Grow(1)
		if ui.Button(c, "刷新").Clicked() {
			s.loadWords()
		}
		ui.SearchField(c, &s.wordQuery).Placeholder("搜索词或拼音").Width(240)
	})
	p := colors(c)
	ui.Column(c).Margin(8, 0, 0, 0).Radius(8).Background(p.card).Border(1, p.border).Clip().Children(func() {
		if !s.wordsLoaded {
			row(c, "正在读取…", "", nil)
			return
		}
		if len(words) == 0 {
			row(c, "没有学到的词", "", nil)
			return
		}
		ui.List(c, nil, len(words), func(i int) {
			w := words[i]
			ui.Row(c).Padding(9, 18).Gap(14).AlignItems(ui.Center).BorderWidth(0, 0, 1, 0).BorderColor(p.border).Children(func() {
				ui.Text(c, w.Word).Width(160).SingleLine()
				ui.Text(c, w.Pinyin).Grow(1).SingleLine().TextColor(p.muted)
				ui.Textf(c, "%.0f", w.Freq).FontSize(12.5).TextColor(p.muted).Width(70)
				if ui.Button(c, "删除").Label("删除 " + w.Word).Clicked() {
					s.forgetWords([]string{w.Word})
				}
			})
		}).MaxHeight(460)
	})
}

func (s *settings) aboutPage(c *ui.Context) {
	pageHeader(c, "关于", "")
	section(c, "", func() {
		row(c, "FreeType "+s.version, "基于 WowType 的 Windows 中文输入法；设置界面使用 mygo。这是测试版本", nil)
		if s.status.DataDir != "" {
			dir := s.status.DataDir
			row(c, "用户数据", dir, func() {
				if ui.Button(c, "打开文件夹").Label("打开用户数据文件夹").Clicked() && s.openFolder != nil {
					s.openFolder(dir)
				}
			})
		}
		if s.status.LogDir != "" {
			dir := s.status.LogDir
			row(c, "日志", "输入法出问题时看这里的 server.log 和 tsf.log", func() {
				if ui.Button(c, "打开文件夹").Label("打开日志文件夹").Clicked() && s.openFolder != nil {
					s.openFolder(dir)
				}
			})
		}
		row(c, "问题反馈", "GitHub 仓库 OUBIGFA/FreeType", nil)
	})
	section(c, "来源与许可", func() {
		row(c, "拼音引擎", "WowType（Gloridust/WowType），保留原有提交历史", nil)
		row(c, "词库", "雾凇拼音（rime-ice，GPL-3.0）；词级语言模型 CC BY-SA 4.0。见安装目录 data 下的 LICENSE 与 SOURCE.txt", nil)
		row(c, "水杉输入法格式", "词库、快捷短语按 msime-windows 公开的文件格式读写，没有使用它的代码和词库", nil)
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
