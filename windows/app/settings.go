package main

import (
	"fmt"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/egoist/mygo/ui"
)

var pageSizes = []string{"3", "4", "5", "6", "7", "8", "9", "10"}
var fontSizes = []string{"14", "16", "18", "20", "22", "24", "28"}

// settings 是设置窗口的状态。界面在主线程上画；找服务进程的事放到 goroutine 里做，
// 做完用 apply 回到主线程改状态。
type settings struct {
	backend Backend
	version string
	// apply 在主线程上执行 fn 并重画；测试里直接执行。
	apply func(fn func())
	// inline：后台的事也在当前线程做完（测试用）。
	inline bool
	// 选文件、存文件的对话框（测试里换掉）。
	openFile func(title string) string
	saveFile func(title, name string) string

	page      string
	loaded    bool
	status    Status
	statusErr string

	config   Config
	pageSize string
	fontSize string
	scheme   string

	dicts   []Dictionary
	phrases []QuickPhrase

	newCode, newText, newWeight string
	formError                   string
	replacePhrases              bool

	// 最近一次导入的结果。
	report       string
	reportErrors []string
	// 下一帧弹出的提示。
	notice string
}

func newSettings(b Backend, version string) *settings {
	return &settings{backend: b, version: version, page: "general", apply: func(fn func()) { fn() }}
}

// background 在后台做 work，做完把它返回的改动交给主线程。
func (s *settings) background(work func() func()) {
	if s.inline {
		if change := work(); change != nil {
			change()
		}
		return
	}
	go func() {
		if change := work(); change != nil {
			s.apply(change)
		}
	}()
}

func schemeName(id string) string {
	for _, sc := range schemes {
		if sc.ID == id {
			return sc.Name
		}
	}
	return schemes[0].Name
}

func schemeID(name string) string {
	for _, sc := range schemes {
		if sc.Name == name {
			return sc.ID
		}
	}
	return schemes[0].ID
}

func schemeNames() []string {
	names := make([]string, len(schemes))
	for i, sc := range schemes {
		names[i] = sc.Name
	}
	return names
}

func (s *settings) setConfig(c Config) {
	s.config = c
	s.pageSize = strconv.Itoa(c.PageSize)
	s.fontSize = strconv.Itoa(c.FontSize)
	s.scheme = schemeName(c.ShuangpinScheme)
	s.loaded = true
}

// refresh 重新读状态、设置、词库和快捷短语。
func (s *settings) refresh() {
	s.background(func() func() {
		status, err := s.backend.Call(op("status"))
		if err != nil {
			return func() { s.statusErr = err.Error() }
		}
		config, cerr := s.backend.Call(op("get_config"))
		dicts, _ := s.backend.Call(op("list_dictionaries"))
		phrases, _ := s.backend.Call(op("list_quick_phrases"))
		return func() {
			s.statusErr = ""
			if status.Status != nil {
				s.status = *status.Status
			}
			if cerr == nil && config.Config != nil {
				s.setConfig(*config.Config)
			}
			s.dicts = dicts.Dictionaries
			s.phrases = phrases.QuickPhrases
		}
	})
}

func (s *settings) saveConfig() {
	c := s.config
	c.PageSize, _ = strconv.Atoi(s.pageSize)
	c.FontSize, _ = strconv.Atoi(s.fontSize)
	c.ShuangpinScheme = schemeID(s.scheme)
	s.background(func() func() {
		resp, err := s.backend.Call(op("set_config", "config", c))
		return func() {
			if err != nil {
				s.notice = "保存失败：" + err.Error()
				return
			}
			if resp.Config != nil {
				s.setConfig(*resp.Config)
			}
		}
	})
}

func describe(r *ImportReport, what string) (string, []string) {
	if r == nil {
		return what + "完成", nil
	}
	msg := fmt.Sprintf("%s %d 条", what, r.Added)
	if r.Skipped > 0 {
		msg += fmt.Sprintf("，跳过 %d 行", r.Skipped)
	}
	return msg, r.Errors
}

func (s *settings) importDictionary() {
	s.background(func() func() {
		path := s.openFile("导入词库（水杉格式）")
		if path == "" {
			return nil
		}
		name := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
		resp, err := s.backend.Call(op("import_dictionary", "path", path, "name", name))
		list, _ := s.backend.Call(op("list_dictionaries"))
		return func() {
			if err != nil {
				s.report, s.reportErrors = "导入失败："+err.Error(), nil
			} else {
				s.report, s.reportErrors = describe(resp.Report, "「"+name+"」导入")
			}
			s.dicts = list.Dictionaries
		}
	})
}

func (s *settings) removeDictionary(name string) {
	s.background(func() func() {
		_, err := s.backend.Call(op("remove_dictionary", "name", name))
		list, _ := s.backend.Call(op("list_dictionaries"))
		return func() {
			if err != nil {
				s.notice = "删除失败：" + err.Error()
			} else {
				s.notice = "已删除「" + name + "」"
			}
			s.dicts = list.Dictionaries
		}
	})
}

func (s *settings) export(request, title, name string) {
	s.background(func() func() {
		path := s.saveFile(title, name)
		if path == "" {
			return nil
		}
		resp, err := s.backend.Call(op(request, "path", path))
		return func() {
			if err != nil {
				s.notice = "导出失败：" + err.Error()
				return
			}
			count := 0
			if resp.Report != nil {
				count = resp.Report.Added
			}
			s.notice = fmt.Sprintf("已导出 %d 条", count)
		}
	})
}

func (s *settings) importPhrases() {
	replace := s.replacePhrases
	s.background(func() func() {
		path := s.openFile("导入快捷短语（水杉格式）")
		if path == "" {
			return nil
		}
		resp, err := s.backend.Call(op("import_quick_phrases", "path", path, "replace", replace))
		list, _ := s.backend.Call(op("list_quick_phrases"))
		return func() {
			if err != nil {
				s.report, s.reportErrors = "导入失败："+err.Error(), nil
			} else {
				s.report, s.reportErrors = describe(resp.Report, "快捷短语新增")
			}
			s.phrases = list.QuickPhrases
		}
	})
}

// savePhrases 整份替换快捷短语。
func (s *settings) savePhrases(phrases []QuickPhrase, done string) {
	s.background(func() func() {
		_, err := s.backend.Call(op("set_quick_phrases", "phrases", phrases))
		list, _ := s.backend.Call(op("list_quick_phrases"))
		return func() {
			if err != nil {
				s.notice = "保存失败：" + err.Error()
			} else {
				s.notice = done
			}
			s.phrases = list.QuickPhrases
		}
	})
}

// validCode 判断快捷短语编码：只能是英文字母。
func validCode(code string) bool {
	if code == "" {
		return false
	}
	for _, r := range code {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z') {
			return false
		}
	}
	return true
}

func (s *settings) addPhrase() {
	code := strings.ToLower(strings.TrimSpace(s.newCode))
	text := strings.ReplaceAll(s.newText, "\r\n", "\n")
	weight := 10
	if w := strings.TrimSpace(s.newWeight); w != "" {
		n, err := strconv.Atoi(w)
		if err != nil || n < 0 {
			s.formError = "权重必须是非负整数。"
			return
		}
		weight = n
	}
	switch {
	case !validCode(code):
		s.formError = "编码只能包含英文字母。"
		return
	case strings.TrimSpace(text) == "":
		s.formError = "短语不能为空。"
		return
	}
	s.formError = ""
	phrases := append(append([]QuickPhrase(nil), s.phrases...), QuickPhrase{Code: code, Text: text, Weight: weight})
	s.newCode, s.newText, s.newWeight = "", "", ""
	s.savePhrases(phrases, "已添加「"+code+"」")
}

func (s *settings) removePhrase(i int) {
	if i < 0 || i >= len(s.phrases) {
		return
	}
	phrases := append(append([]QuickPhrase(nil), s.phrases[:i]...), s.phrases[i+1:]...)
	s.savePhrases(phrases, "已删除「"+s.phrases[i].Code+"」")
}

func (s *settings) toggleMode() {
	s.background(func() func() {
		resp, err := s.backend.Call(op("toggle_mode"))
		return func() {
			if err == nil && resp.Frame != nil {
				s.status.Mode = resp.Frame.Mode
			}
		}
	})
}

// preview 是短语在列表里的样子：只显示第一行，后面还有行时加 ↵。
func preview(text string) string {
	first, _, more := strings.Cut(text, "\n")
	if more {
		return first + " ↵"
	}
	return first
}

func (s *settings) view(c *ui.Context) {
	if s.notice != "" {
		c.Toast(s.notice)
		s.notice = ""
	}
	ui.Row(c).Fill().Children(func() {
		ui.Sidebar(c, &s.page, func() {
			ui.SidebarSection(c, "FreeType", nil, func() {
				ui.SidebarItem(c, "general", nil, "常规")
				ui.SidebarItem(c, "dicts", nil, "词库")
				ui.SidebarItem(c, "phrases", nil, "快捷短语")
				ui.SidebarItem(c, "about", nil, "关于")
			})
		}).Width(168)
		ui.Scroll(c).Grow(1).Padding(20, 28).Gap(14).Children(func() {
			switch s.page {
			case "dicts":
				s.dictsPage(c)
			case "phrases":
				s.phrasesPage(c)
			case "about":
				s.aboutPage(c)
			default:
				s.generalPage(c)
			}
		})
	})
}

func title(c *ui.Context, text string) {
	ui.Text(c, text).FontSize(22).Bold()
}

func note(c *ui.Context, text string) {
	ui.Text(c, text).TextColor(c.Theme().TextMuted).Wrap()
}

func (s *settings) statusLine(c *ui.Context) {
	t := c.Theme()
	switch {
	case s.statusErr != "":
		ui.Text(c, s.statusErr).TextColor(t.Danger).Wrap()
	case s.status.Error != "":
		ui.Text(c, "词库载入失败："+s.status.Error).TextColor(t.Danger).Wrap()
	case !s.status.Ready:
		ui.Text(c, "输入法服务正在载入词库…").TextColor(t.TextMuted)
	default:
		mode := "中文"
		if s.status.Mode == "english" {
			mode = "英文"
		}
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			ui.Text(c, "输入法服务运行中，当前为"+mode+"状态").TextColor(t.Success)
			if ui.Button(c, "切换中 / 英").Clicked() {
				s.toggleMode()
			}
		})
	}
}

func (s *settings) generalPage(c *ui.Context) {
	title(c, "常规")
	s.statusLine(c)
	if !s.loaded {
		return
	}
	changed := false
	sw := func(label string, value *bool) {
		ui.Field(c, label, func() {
			if ui.Switch(c, value).Changed() {
				changed = true
			}
		})
	}
	sel := func(label string, value *string, options []string, disabled bool) {
		ui.Field(c, label, func() {
			if ui.Select(c, value, options).Width(160).Disabled(disabled).Changed() {
				changed = true
			}
		})
	}
	ui.Form(c, func() {
		ui.Fieldset(c, "候选窗", func() {
			sel("每页候选数", &s.pageSize, pageSizes, false)
			sel("候选字号", &s.fontSize, fontSizes, false)
			sw("横排显示", &s.config.Horizontal)
		})
		ui.Fieldset(c, "拼音", func() {
			sw("模糊音", &s.config.Fuzzy)
			sw("按错键纠错", &s.config.TypoCorrection)
			sw("双拼", &s.config.Shuangpin)
			sel("双拼方案", &s.scheme, schemeNames(), !s.config.Shuangpin)
		})
		ui.Fieldset(c, "输出", func() {
			sw("中文标点用全角", &s.config.FullwidthPunct)
			sw("输出繁体", &s.config.Traditional)
		})
		ui.Fieldset(c, "实用功能", func() {
			sw("快捷短语混入候选", &s.config.QuickPhrases)
			sw("推荐 emoji", &s.config.EmojiSuggest)
			sw("日期时间（riqi、shijian）", &s.config.DateTime)
			sw("V 模式计算器", &s.config.Calculator)
		})
	})
	if changed {
		s.saveConfig()
	}
	note(c, "单按 Shift 切换中文 / 英文。修改立即生效。")
}

func (s *settings) reportView(c *ui.Context) {
	if s.report == "" {
		return
	}
	ui.Text(c, s.report).Bold().Wrap()
	for _, e := range s.reportErrors {
		ui.Text(c, e).TextColor(c.Theme().Danger).Wrap()
	}
}

func (s *settings) dictsPage(c *ui.Context) {
	title(c, "词库")
	note(c, "导入水杉输入法（msime）格式的词库文件：每行「词语<Tab>拼音[<Tab>权重]」，拼音可写成 ni'hao、ni hao 或 nihao，# 开头的行是注释。导入的词参与组词和排序，不会混进你学到的词。")
	ui.Row(c).Gap(10).Children(func() {
		if ui.PrimaryButton(c, "导入词库…").Clicked() {
			s.importDictionary()
		}
		if ui.Button(c, "导出学到的词…").Clicked() {
			s.export("export_user_dictionary", "导出学到的词", "FreeType 用户词库.txt")
		}
	})
	s.reportView(c)
	ui.Divider(c)
	if len(s.dicts) == 0 {
		note(c, "还没有导入词库。")
		return
	}
	for _, d := range s.dicts {
		name := d.Name
		ui.Row(c).Gap(12).AlignItems(ui.Center).Children(func() {
			ui.Text(c, name).Grow(1).SingleLine()
			ui.Textf(c, "%d 条", d.Entries).TextColor(c.Theme().TextMuted)
			if ui.Button(c, "删除").Label("删除 "+name).Clicked() {
				s.removeDictionary(name)
			}
		})
	}
}

func (s *settings) phrasesPage(c *ui.Context) {
	title(c, "快捷短语")
	note(c, `输入的编码（只能是英文字母）正好对上时，短语排在候选第一位。批量文件与水杉输入法相同：每行「编码<Tab>短语[<Tab>权重]」，短语里的换行写成 \n。`)
	ui.Form(c, func() {
		ui.Field(c, "编码", func() { ui.TextInput(c, &s.newCode).Placeholder("例如 dz").Width(200) })
		ui.Field(c, "短语", func() { ui.TextArea(c, &s.newText).Height(72) }).Error(s.formError)
		ui.Field(c, "权重", func() { ui.TextInput(c, &s.newWeight).Placeholder("10").Width(100) }).
			Description("同一编码有多条时，权重大的在前。")
	})
	ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
		if ui.PrimaryButton(c, "添加").Clicked() {
			s.addPhrase()
		}
		if ui.Button(c, "导入…").Clicked() {
			s.importPhrases()
		}
		ui.Checkbox(c, &s.replacePhrases, "导入时替换现有短语")
		if ui.Button(c, "导出…").Clicked() {
			s.export("export_quick_phrases", "导出快捷短语", "FreeType 快捷短语.txt")
		}
	})
	s.reportView(c)
	ui.Divider(c)
	if len(s.phrases) == 0 {
		note(c, "还没有快捷短语。")
		return
	}
	remove := -1
	for i, p := range s.phrases {
		ui.Row(c).Gap(12).AlignItems(ui.Center).Children(func() {
			ui.Text(c, p.Code).Bold().Width(96).SingleLine()
			ui.Text(c, preview(p.Text)).Grow(1).SingleLine()
			ui.Textf(c, "%d", p.Weight).TextColor(c.Theme().TextMuted)
			if ui.Button(c, "删除").Label(fmt.Sprintf("删除 %s %d", p.Code, i+1)).Clicked() {
				remove = i
			}
		})
	}
	if remove >= 0 {
		s.removePhrase(remove)
	}
}

func (s *settings) aboutPage(c *ui.Context) {
	title(c, "关于")
	ui.Text(c, "FreeType "+s.version).Bold()
	note(c, "基于 WowType 的 Windows 中文输入法；界面使用 mygo。")
	if s.status.DataDir != "" {
		note(c, "用户数据："+s.status.DataDir)
	}
	note(c, "随附词库来自雾凇拼音（rime-ice，GPL-3.0），词级语言模型为 CC BY-SA 4.0，见安装目录 data 下的 LICENSE 与 SOURCE.txt。")
	note(c, "这是测试版本，问题请到 GitHub 仓库 OUBIGFA/FreeType 反馈。")
}
