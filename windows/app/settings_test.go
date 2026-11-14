package main

import (
	"encoding/json"
	"fmt"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/egoist/mygo/ui"
)

// fakeBackend 记下请求，按 op 给应答。
type fakeBackend struct {
	calls       []Request
	config      Config
	phrases     []QuickPhrase
	common      []string
	dicts       []Dictionary
	words       []UserWord
	mode        string
	failRequest string
}

func (f *fakeBackend) Call(r Request) (Response, error) {
	f.calls = append(f.calls, r)
	if r["op"] == f.failRequest {
		return Response{}, fmt.Errorf("测试错误")
	}
	switch r["op"] {
	case "status":
		return Response{OK: true, Status: &Status{Ready: true, Mode: f.mode, DataDir: `C:\data`, LogDir: `C:\logs`, Version: "0.0.1"}}, nil
	case "get_config":
		c := f.config
		return Response{OK: true, Config: &c}, nil
	case "set_config":
		// 走一遍 JSON，和真的管道一样
		raw, _ := json.Marshal(r["config"])
		f.config = Config{}
		_ = json.Unmarshal(raw, &f.config)
		c := f.config
		return Response{OK: true, Config: &c}, nil
	case "toggle_mode":
		if f.mode == "english" {
			f.mode = "chinese"
		} else {
			f.mode = "english"
		}
		return Response{OK: true, Frame: &Frame{Mode: f.mode}}, nil
	case "command":
		if r["command"] == "toggle_punct" {
			f.config.FullwidthPunct = !f.config.FullwidthPunct
		}
		return Response{OK: true}, nil
	case "list_quick_phrases":
		return Response{OK: true, QuickPhrases: f.phrases}, nil
	case "set_quick_phrases":
		raw, _ := json.Marshal(r["phrases"])
		f.phrases = nil
		_ = json.Unmarshal(raw, &f.phrases)
		return Response{OK: true}, nil
	case "list_common_phrases":
		return Response{OK: true, CommonPhrases: f.common}, nil
	case "set_common_phrases":
		raw, _ := json.Marshal(r["phrases"])
		_ = json.Unmarshal(raw, &f.common)
		return Response{OK: true}, nil
	case "import_common_phrases":
		f.common = append(f.common, "导入常用语")
		return Response{OK: true, Report: &ImportReport{Added: 1}}, nil
	case "export_common_phrases":
		return Response{OK: true, Report: &ImportReport{Added: len(f.common)}}, nil
	case "list_dictionaries":
		return Response{OK: true, Dictionaries: f.dicts}, nil
	case "import_dictionary":
		f.dicts = append(f.dicts, Dictionary{Name: r["name"].(string), Entries: 2})
		return Response{OK: true, Report: &ImportReport{Added: 2, Skipped: 1, Errors: []string{"第 3 行：编码不是有效的全拼"}}}, nil
	case "list_user_words":
		return Response{OK: true, UserWords: f.words}, nil
	case "export_learning":
		return Response{OK: true, Report: &ImportReport{Added: len(f.words), Note: "包含词条、选词次数和搭配"}}, nil
	case "import_learning":
		f.words = append(f.words, UserWord{Word: "恢复词", Pinyin: "hui fu ci", Freq: 3000})
		return Response{OK: true, UserWords: f.words, Report: &ImportReport{Added: 3}}, nil
	case "export_settings", "inspect_settings_backup":
		return Response{OK: true, Report: &ImportReport{Added: 5, Note: "全部设置、1 份词库、1 条快捷短语、1 条常用语、2 条学到的词及使用记录"}}, nil
	case "import_settings":
		f.config.PageSize, f.config.TrayIcon = 9, false
		f.dicts = []Dictionary{{Name: "恢复词库", Entries: 10}}
		f.phrases = []QuickPhrase{{Code: "hf", Text: "恢复短语", Weight: 20}}
		f.common = []string{"恢复常用语"}
		f.words = []UserWord{{Word: "恢复词", Pinyin: "hui fu ci", Freq: 5000}}
		c := f.config
		return Response{OK: true, Config: &c, Status: &Status{Ready: true, Mode: "chinese"}, Dictionaries: f.dicts, QuickPhrases: f.phrases, CommonPhrases: f.common, UserWords: f.words, Report: &ImportReport{Added: 3}}, nil
	case "forget_words":
		raw, _ := json.Marshal(r["words"])
		var gone []string
		_ = json.Unmarshal(raw, &gone)
		var keep []UserWord
		for _, w := range f.words {
			drop := false
			for _, g := range gone {
				drop = drop || g == w.Word
			}
			if !drop {
				keep = append(keep, w)
			}
		}
		f.words = keep
		return Response{OK: true, UserWords: f.words}, nil
	case "clear_learned":
		f.words = nil
		return Response{OK: true}, nil
	case "add_user_word":
		w := UserWord{Word: r["word"].(string), Pinyin: "shuang pin ge", Freq: 50000}
		f.words = append([]UserWord{w}, f.words...)
		return Response{OK: true, UserWords: f.words, Report: &ImportReport{Added: 1, Note: "霜频阁（shuang pin ge）"}}, nil
	}
	return Response{OK: true}, nil
}

func (f *fakeBackend) last(name string) Request {
	for i := len(f.calls) - 1; i >= 0; i-- {
		if f.calls[i]["op"] == name {
			return f.calls[i]
		}
	}
	return nil
}

func newTestSettings() (*settings, *fakeBackend) {
	b := &fakeBackend{config: defaultConfig(), mode: "chinese"}
	b.config.PageSize = 6
	b.words = []UserWord{{Word: "设置业", Pinyin: "she zhi ye", Freq: 3000}, {Word: "输入法", Pinyin: "shu ru fa", Freq: 3000}}
	b.phrases = []QuickPhrase{{Code: "dz", Text: "北京市\n海淀区", Weight: 10}}
	s := newSettings(b, "0.0.1")
	s.inline = true
	s.refresh()
	return s, b
}

func open(t *testing.T, s *settings, page string) *ui.Tester {
	t.Helper()
	tt := ui.NewTester(s.view, 1000, 1400)
	if page != "" {
		if err := tt.Click(page); err != nil {
			t.Fatalf("打不开「%s」：%v", page, err)
		}
	}
	return tt
}

func TestGeneralPageShowsStatusAndSavesWholeConfig(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "")
	for _, text := range []string{"正在运行", "中英文切换", "悬浮状态栏", "切换输入框时提示", "提示位置"} {
		if !tt.HasText(text) {
			t.Fatalf("没有「%s」：%q", text, tt.Texts())
		}
	}
	// 点名字就能切换开关
	if err := tt.Click("切换输入框时提示"); err != nil {
		t.Fatal(err)
	}
	if b.last("set_config") == nil || !b.config.HintOnFocus {
		t.Fatalf("开关没有保存：%+v", b.config)
	}
	if b.config.PageSize != 6 || b.config.FontFamily != "Microsoft YaHei" || !b.config.AutoPair {
		t.Fatalf("别的设置被改了：%+v", b.config)
	}
	if err := tt.Click("英文"); err != nil {
		t.Fatal(err)
	}
	if b.last("toggle_mode") == nil || s.status.Mode != "english" {
		t.Fatalf("没有切到英文：%+v", s.status)
	}
	// 状态栏默认关：打开后才显示「自动隐藏」
	if tt.HasText("自动隐藏") || b.config.StatusBar {
		t.Fatal("状态栏默认应关着")
	}
	if err := tt.Click("悬浮状态栏"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("自动隐藏") || !b.config.StatusBar {
		t.Fatalf("打开状态栏后应显示子项：%+v", b.config)
	}
	if b.config.HintPosition != "top-left" || !b.config.HintOnToggle || !b.config.Autospace || !b.config.AutoPair {
		t.Fatalf("默认值不对：%+v", b.config)
	}
}

func TestGlobalDefaultChoiceSavesAndHidesConflictingScope(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "")
	chooseDefault := func(value string) {
		t.Helper()
		for _, control := range tt.AccessAll() {
			if control.Name == "默认状态" && control.Focusable {
				r := control.Bounds
				tt.ClickAt(r.X+r.W/2, r.Y+r.H/2)
				if err := tt.Click(value); err != nil {
					t.Fatal(err)
				}
				return
			}
		}
		t.Fatal("没有默认状态下拉框")
	}
	chooseDefault("全局")
	tt.Frame()
	if s.config.DefaultMode != "global" || b.config.DefaultMode != "global" {
		t.Fatalf("全局选择没有保存：%+v", s.config)
	}
	if _, found := tt.Find("中英文状态"); found {
		t.Fatal("全局状态下不应再显示按应用记忆的冲突选项")
	}
	if err := tt.Click("切换输入框时提示"); err != nil {
		t.Fatal(err)
	}
	s.refresh()
	tt.Frame()
	if s.config.DefaultMode != "global" {
		t.Fatal("保存其他设置后丢失了全局选择")
	}
	chooseDefault("英文")
	tt.Frame()
	if _, found := tt.Find("中英文状态"); !found || s.config.DefaultMode != "english" {
		t.Fatal("固定英文默认状态应仍允许按应用记忆")
	}
}

func TestTypingPageFuzzyGroups(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "拼音与按键")
	if tt.HasText("n = l") {
		t.Fatal("模糊音关着时不显示分组")
	}
	if err := tt.Click("模糊音"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.Click("n = l"); err != nil {
		t.Fatal(err)
	}
	if !b.config.Fuzzy || b.config.FuzzyGroups&8 == 0 || b.config.FuzzyGroups&1 == 0 {
		t.Fatalf("模糊音分组没有保存：%+v", b.config)
	}
	if err := tt.Click("成对标点自动补全"); err != nil {
		t.Fatal(err)
	}
	if b.config.AutoPair {
		t.Fatal("成对标点没有关掉")
	}
}

func TestCandidatesPagePreviewsFont(t *testing.T) {
	fontNames = func() []string { return []string{"SimSun", "Arial", "Microsoft YaHei"} }
	s, _ := newTestSettings()
	tt := open(t, s, "候选窗")
	for _, text := range []string{"预览", "你好", "字体", "字号", "排列方向"} {
		if !tt.HasText(text) {
			t.Fatalf("没有「%s」：%q", text, tt.Texts())
		}
	}
	if got := s.fontList(fontNames); got[0] != "Microsoft YaHei" || got[1] != "SimSun" {
		t.Fatalf("常用中文字体应在前：%q", got)
	}
	// 预览按每页个数、字体实时变
	if !tt.HasText("妮") || tt.HasText("呢") {
		t.Fatalf("预览没按每页 6 个画：%q", tt.Texts())
	}
	s.config.PageSize, s.config.FontFamily = 8, "SimSun"
	tt.Frame()
	if !tt.HasText("呢") {
		t.Fatalf("预览没跟着变：%q", tt.Texts())
	}
}

func TestShortcutsPageSavesHotkey(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "快捷键")
	if !tt.HasText("Ctrl + Delete") || !tt.HasText("可以设置的快捷键") {
		t.Fatalf("快捷键页内容不全：%q", tt.Texts())
	}
	s.config.HotkeyPunct = "ctrl+period"
	s.saveConfig()
	if b.config.HotkeyPunct != "ctrl+period" {
		t.Fatalf("快捷键没有保存：%+v", b.config)
	}
}

func TestDictionaryImportShowsReport(t *testing.T) {
	s, b := newTestSettings()
	s.openFile = func(string) string { return `C:\dl\常用词.txt` }
	tt := open(t, s, "词库")
	if err := tt.Click("导入词库…"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.last("import_dictionary")["name"] != "常用词" {
		t.Fatalf("词库名不对：%v", b.last("import_dictionary"))
	}
	if !tt.HasText("「常用词」导入 2 条，跳过 1 行") || !tt.HasText("第 3 行：编码不是有效的全拼") {
		t.Fatalf("没有显示导入结果：%q", tt.Texts())
	}
}

func TestPhrasesAddValidateAndRemove(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "快捷短语")
	if !tt.HasText("北京市 ↵") {
		t.Fatalf("多行短语应显示第一行：%q", tt.Texts())
	}
	s.newCode, s.newText = "a1", "x"
	s.addPhrase()
	if s.formError == "" {
		t.Fatal("编码带数字应报错")
	}
	s.newCode, s.newText, s.newWeight = "QM", "签名", "20"
	s.addPhrase()
	if len(b.phrases) != 2 || b.phrases[1].Code != "qm" || b.phrases[1].Weight != 20 {
		t.Fatalf("没有加上：%+v", b.phrases)
	}
	s.phraseQuery = "签名"
	if got := s.filteredPhrases(); len(got) != 1 || got[0] != 1 {
		t.Fatalf("搜索结果不对：%v", got)
	}
	s.removePhrase(0)
	if len(b.phrases) != 1 || b.phrases[0].Code != "qm" {
		t.Fatalf("没有删掉：%+v", b.phrases)
	}
}

func TestPhraseListShowsAtMostTwelveRows(t *testing.T) {
	s, b := newTestSettings()
	for i := 0; i < 30; i++ {
		b.phrases = append(b.phrases, QuickPhrase{Code: "zz", Text: fmt.Sprintf("短语%02d", i), Weight: 1})
	}
	s.refresh()
	tt := ui.NewTester(s.view, 1000, 3000)
	if err := tt.Click("快捷短语"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("短语05") || tt.HasText("短语20") {
		t.Fatalf("列表应只显示前面几行，其余滚动：%q", tt.Texts())
	}
}

func TestCommonPhrasesPageAddsMultilineAndRemoves(t *testing.T) {
	s, b := newTestSettings()
	b.common = []string{"原有常用语"}
	s.refresh()
	tt := open(t, s, "常用语")
	if err := tt.Click("常用语内容"); err != nil {
		t.Fatal(err)
	}
	tt.Type("第一行\n第二行FreeType")
	if err := tt.Click("添加常用语"); err != nil {
		t.Fatal(err)
	}
	if len(b.common) != 2 || b.common[1] != "第一行\n第二行FreeType" {
		t.Fatalf("应保存完整多行常用语：%v", b.common)
	}
	if !tt.HasText("第一行 ↵") {
		t.Fatalf("应显示多行预览：%q", tt.Texts())
	}
	if err := tt.Click("删除常用语 1"); err != nil {
		t.Fatal(err)
	}
	if len(b.common) != 1 || b.common[0] != "第一行\n第二行FreeType" {
		t.Fatalf("删除错误：%v", b.common)
	}
	if len(b.phrases) != 1 || b.phrases[0].Code != "dz" {
		t.Fatal("常用语管理不应改动快捷短语")
	}
}

func TestCommonPhrasesPageImportsAndExportsJson(t *testing.T) {
	s, b := newTestSettings()
	s.openFile = func(title string) string {
		if title != "导入常用语" {
			t.Fatalf("文件选择标题：%s", title)
		}
		return `C:\test\common.json`
	}
	s.saveFile = func(title, name string) string {
		if title != "导出常用语" || name != "FreeType 常用语.json" {
			t.Fatalf("导出格式：%s %s", title, name)
		}
		return `C:\test\export.json`
	}
	tt := open(t, s, "常用语")
	if err := tt.Click("导入…"); err != nil {
		t.Fatal(err)
	}
	if b.last("import_common_phrases")["path"] != `C:\test\common.json` || !tt.HasText("导入常用语") {
		t.Fatalf("应导入并刷新常用语：%v %q", b.calls, tt.Texts())
	}
	if err := tt.Click("导出…"); err != nil {
		t.Fatal(err)
	}
	if b.last("export_common_phrases")["path"] != `C:\test\export.json` {
		t.Fatal("未导出常用语")
	}
}

func TestFullSettingsImportNeedsConfirmationAndRefreshesAllData(t *testing.T) {
	s, b := newTestSettings()
	s.openFile = func(title string) string {
		if title != "导入全部设置" {
			t.Fatalf("导入标题：%s", title)
		}
		return `C:\test\settings.json`
	}
	var tray bool = true
	s.onTray = func(show bool) { tray = show }
	tt := open(t, s, "常规")
	if err := tt.Click("导入全部设置"); err != nil {
		t.Fatal(err)
	}
	if b.last("inspect_settings_backup") == nil || b.last("import_settings") != nil {
		t.Fatal("确认前只能校验备份")
	}
	if err := tt.Click("导入并替换"); err != nil {
		t.Fatal(err)
	}
	if b.last("import_settings")["path"] != `C:\test\settings.json` {
		t.Fatal("导入路径错误")
	}
	if s.config.PageSize != 9 || tray || s.dicts[0].Name != "恢复词库" || s.phrases[0].Text != "恢复短语" || s.common[0] != "恢复常用语" || s.words[0].Word != "恢复词" {
		t.Fatalf("导入后必须刷新全部数据与托盘开关：%+v", s)
	}
}

func TestFullSettingsImportCancelAndInvalidFilePreserveCurrentData(t *testing.T) {
	for _, invalid := range []bool{false, true} {
		s, b := newTestSettings()
		s.openFile = func(string) string { return `C:\test\settings.json` }
		if invalid {
			b.failRequest = "inspect_settings_backup"
		}
		tt := open(t, s, "常规")
		if err := tt.Click("导入全部设置"); err != nil {
			t.Fatal(err)
		}
		if !invalid {
			if err := tt.Click("取消"); err != nil {
				t.Fatal(err)
			}
		}
		if b.last("import_settings") != nil || s.config.PageSize != 6 || len(s.phrases) != 1 {
			t.Fatal("取消或无效备份不能更改当前数据")
		}
	}
}

func TestFullSettingsExportUsesPortableJson(t *testing.T) {
	s, b := newTestSettings()
	s.saveFile = func(title, name string) string {
		if title != "导出全部设置" || name != "FreeType 全部设置.json" {
			t.Fatalf("导出信息错误：%s %s", title, name)
		}
		return `C:\test\all.json`
	}
	tt := open(t, s, "常规")
	if err := tt.Click("导出全部设置"); err != nil {
		t.Fatal(err)
	}
	if b.last("export_settings")["path"] != `C:\test\all.json` {
		t.Fatal("必须导出完整备份")
	}
}

func TestSettingsSearchLocatesMatchingRowAcrossPages(t *testing.T) {
	s, _ := newTestSettings()
	tt := ui.NewTester(s.view, 1000, 540)
	if err := tt.Click("搜索全部设置"); err != nil {
		t.Fatal(err)
	}
	tt.Type("自动加空格")
	if err := tt.Click("定位 拼音与按键 / 中英文之间自动加空格"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if s.page != "typing" {
		t.Fatalf("应跳转到拼音与按键：%s", s.page)
	}
	bounds, found := tt.Find("设置项：中英文之间自动加空格")
	if !found || bounds.Y < 32 || bounds.Y+bounds.H > 540 || bounds.X < 236 {
		t.Fatalf("应滚动到目标设置：%+v，存在=%v", bounds, found)
	}
}

func TestSettingsSearchRevealsHiddenOptionsWithoutChangingConfig(t *testing.T) {
	for _, title := range []string{"汉字和字母之间", "双拼方案", "自动隐藏", "中英文状态"} {
		s, b := newTestSettings()
		s.config.Autospace, s.config.Shuangpin, s.config.StatusBar = false, false, false
		s.config.DefaultMode = "global"
		tt := ui.NewTester(s.view, 1000, 540)
		if err := tt.Click("搜索全部设置"); err != nil {
			t.Fatal(err)
		}
		tt.Type(title)
		page := "拼音与按键"
		if title == "自动隐藏" || title == "中英文状态" {
			page = "常规"
		}
		if err := tt.Click("定位 " + page + " / " + title); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
		bounds, found := tt.Find("设置项：" + title)
		if !found || bounds.Y < 32 || bounds.Y+bounds.H > 540 {
			t.Fatalf("应能定位隐藏选项 %s：%+v", title, bounds)
		}
		if s.config.Autospace || s.config.Shuangpin || s.config.StatusBar || s.config.DefaultMode != "global" || b.last("set_config") != nil {
			t.Fatal("搜索不能自行改变任何设置")
		}
	}
}

func TestSettingsSearchNoMatchesEscapeAndMinimumWindow(t *testing.T) {
	s, _ := newTestSettings()
	tt := ui.NewTester(s.view, 780, 540)
	tt.Frame()
	bounds, found := tt.Find("搜索全部设置")
	if !found || bounds.X < 0 || bounds.X+bounds.W > 236 || bounds.Y+bounds.H > 540 {
		t.Fatalf("侧边栏底部应显示搜索框：%+v", bounds)
	}
	if err := tt.Click("搜索全部设置"); err != nil {
		t.Fatal(err)
	}
	tt.Type("不存在的设置xyz")
	if !tt.HasText("没有找到相关设置") {
		t.Fatalf("应显示无结果提示：%q", tt.Texts())
	}
	tt.Key(0, ui.KeyEscape)
	if !tt.HasText("中英文切换") {
		t.Fatal("Esc 应退出搜索，返回当前页面")
	}
}

func TestEverySettingsSearchResultHasARealTarget(t *testing.T) {
	for _, item := range settingsItems {
		t.Run(item.ID, func(t *testing.T) {
			s, _ := newTestSettings()
			s.locateSetting(item)
			tt := ui.NewTester(s.view, 1000, 1400)
			tt.Frame()
			if _, found := tt.Find("设置项：" + item.Title); !found {
				t.Fatalf("搜索结果没有对应的实际设置：%s", item.ID)
			}
		})
	}
}

func TestAddWordWithAndWithoutPinyin(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "学到的词")
	s.addWord()
	if s.wordError == "" {
		t.Fatal("没填词语应提示")
	}
	s.newWord, s.newPinyin = "霜频阁", ""
	if err := tt.Click("添加"); err != nil {
		t.Fatal(err)
	}
	req := b.last("add_user_word")
	if req == nil || req["word"] != "霜频阁" || req["pinyin"] != "" {
		t.Fatalf("请求不对：%v", req)
	}
	if s.newWord != "" || s.wordError != "" {
		t.Fatalf("加完应清空输入：%q %q", s.newWord, s.wordError)
	}
}

func TestLearnedWordsSearchDeleteAndClear(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "学到的词")
	tt.Frame()
	if !tt.HasText("设置业") || !tt.HasText("学到的词（2）") {
		t.Fatalf("没有列出学到的词：%q", tt.Texts())
	}
	s.wordQuery = "shezhi"
	if got := s.filteredWords(); len(got) != 1 || got[0].Word != "设置业" {
		t.Fatalf("按拼音搜索不对：%+v", got)
	}
	s.wordQuery = ""
	tt.Frame()
	if err := tt.Click("删除"); err != nil {
		t.Fatal(err)
	}
	if b.last("forget_words") == nil || len(b.words) != 1 {
		t.Fatalf("没有删掉：%+v", b.words)
	}
	if err := tt.Click("全部清空…"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.Click("清空"); err != nil {
		t.Fatal(err)
	}
	if b.last("clear_learned") == nil || len(s.words) != 0 {
		t.Fatalf("没有清空：%+v", s.words)
	}
}

func TestLearnedMixedWordsDisplaySearchAndDeleteTheOriginalEntry(t *testing.T) {
	s, b := newTestSettings()
	if err := json.Unmarshal([]byte(`[{"word":"打tag","display":"打 tag","pinyin":"datag","freq":3000}]`), &b.words); err != nil {
		t.Fatal(err)
	}
	tt := open(t, s, "学到的词")
	if !tt.HasText("打 tag") {
		t.Fatalf("应显示上屏的混输格式：%q", tt.Texts())
	}
	s.wordQuery = "打 tag"
	tt.Frame()
	if !tt.HasText("打 tag") {
		t.Fatal("按上屏内容搜索应能找到混输词")
	}
	if err := tt.Click("删除 打 tag"); err != nil {
		t.Fatal(err)
	}
	if len(b.words) != 0 {
		t.Fatal("显示格式不能改变真正删除的词条标识")
	}
	deleted, _ := json.Marshal(b.last("forget_words")["words"])
	if string(deleted) != `["打tag"]` {
		t.Fatalf("删除标识错误：%s", deleted)
	}
}

func TestLearningBackupExportUsesJsonAndNotDictionaryExport(t *testing.T) {
	s, backend := newTestSettings()
	var suggested string
	s.saveFile = func(title, name string) string {
		suggested = name
		return "C:/backup/learning.json"
	}
	tester := open(t, s, "学到的词")
	if err := tester.Click("导出…"); err != nil {
		t.Fatal(err)
	}
	if filepath.Ext(suggested) != ".json" {
		t.Fatalf("完整备份应保存为 JSON：%s", suggested)
	}
	request := backend.last("export_learning")
	if request == nil || request["path"] != "C:/backup/learning.json" {
		t.Fatalf("没有请求导出完整学习记录：%v", request)
	}
	if backend.last("export_user_dictionary") != nil {
		t.Fatal("完整备份不能只导出词表")
	}
	if !tester.HasText("已导出 2 条（包含词条、选词次数和搭配）") {
		t.Fatalf("导出结果应说明备份包含输入习惯：%q", tester.Texts())
	}
}

func TestLearningBackupExportCancelDoesNotCallServer(t *testing.T) {
	s, backend := newTestSettings()
	s.saveFile = func(string, string) string { return "" }
	tester := open(t, s, "学到的词")
	if err := tester.Click("导出…"); err != nil {
		t.Fatal(err)
	}
	if backend.last("export_learning") != nil {
		t.Fatal("取消文件选择后不应执行导出")
	}
}

func TestLearningBackupImportRefreshesWords(t *testing.T) {
	state, backend := newTestSettings()
	state.openFile = func(string) string { return "C:/backup/learning.json" }
	tester := open(t, state, "学到的词")
	if err := tester.Click("导入…"); err != nil {
		t.Fatal(err)
	}
	request := backend.last("import_learning")
	if request == nil || request["path"] != "C:/backup/learning.json" {
		t.Fatalf("没有请求导入完整学习记录：%v", request)
	}
	tester.Frame()
	if !tester.HasText("恢复词") || !tester.HasText("学到的词（3）") {
		t.Fatalf("导入后应刷新词条：%q", tester.Texts())
	}
	if !strings.Contains(strings.Join(tester.Texts(), " "), "学习记录已合并 3 条") {
		t.Fatalf("导入结果应报告合并的记录数：%q", tester.Texts())
	}
}

func TestLearningBackupImportCancelDoesNotCallServer(t *testing.T) {
	state, backend := newTestSettings()
	state.openFile = func(string) string { return "" }
	tester := open(t, state, "学到的词")
	if err := tester.Click("导入…"); err != nil {
		t.Fatal(err)
	}
	if backend.last("import_learning") != nil {
		t.Fatal("取消文件选择后不应执行导入")
	}
}

func TestLearningBackupErrorsPreserveWords(t *testing.T) {
	for _, test := range []struct{ operation, button string }{
		{"export_learning", "导出…"},
		{"import_learning", "导入…"},
	} {
		t.Run(test.operation, func(t *testing.T) {
			state, backend := newTestSettings()
			backend.failRequest = test.operation
			state.openFile = func(string) string { return "C:/backup/learning.json" }
			state.saveFile = func(string, string) string { return "C:/backup/learning.json" }
			tester := open(t, state, "学到的词")
			if err := tester.Click(test.button); err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(strings.Join(tester.Texts(), " "), "测试错误") || len(state.words) != 2 {
				t.Fatalf("失败应显示原因并保留词条：%q %+v", tester.Texts(), state.words)
			}
		})
	}
}

func TestLearningBackupButtonsFitMinimumWidth(t *testing.T) {
	state, _ := newTestSettings()
	state.page = "learned"
	state.loadWords()
	tester := ui.NewTester(state.view, 780, 540)
	tester.Frame()
	for _, label := range []string{"导入…", "导出…", "全部清空…"} {
		bounds, found := tester.Find(label)
		if !found || bounds.X < 0 || bounds.X+bounds.W > 780 || bounds.Y+bounds.H > 540 {
			t.Fatalf("最小窗口中应能看到并使用 %s：%+v", label, bounds)
		}
	}
	count := 0
	for _, label := range tester.Texts() {
		if label == "导出…" {
			count++
		}
	}
	if count != 1 {
		t.Fatalf("学习页只保留一个导出按钮，实际 %d 个", count)
	}
	if directory := os.Getenv("FREETYPE_SNAPSHOT_DIR"); directory != "" {
		file, err := os.Create(filepath.Join(directory, "learned-min-width.png"))
		if err != nil {
			t.Fatal(err)
		}
		defer file.Close()
		if err := png.Encode(file, tester.Image()); err != nil {
			t.Fatal(err)
		}
	}
}

func TestAboutPageOpensFolders(t *testing.T) {
	s, _ := newTestSettings()
	var opened []string
	s.openFolder = func(path string) { opened = append(opened, path) }
	tt := open(t, s, "关于")
	if err := tt.Click("打开文件夹"); err != nil {
		t.Fatal(err)
	}
	if len(opened) != 1 || opened[0] != `C:\data` {
		t.Fatalf("打开的文件夹不对：%v", opened)
	}
}

// 设了 FREETYPE_SNAPSHOT_DIR 时把每一页画成 PNG（看设计用，平时跳过）。
func TestSnapshots(t *testing.T) {
	dir := os.Getenv("FREETYPE_SNAPSHOT_DIR")
	if dir == "" {
		t.Skip("FREETYPE_SNAPSHOT_DIR 没设")
	}
	fontNames = func() []string { return []string{"Microsoft YaHei", "SimSun", "KaiTi"} }
	// 候选窗预览换成楷体、每页 9 个：看预览是不是真的跟着变
	{
		s, _ := newTestSettings()
		s.page, s.config.FontFamily, s.config.PageSize = "candidates", "KaiTi", 9
		tt := ui.NewTester(s.view, 1400, 720)
		tt.Frame()
		f, err := os.Create(filepath.Join(dir, "candidates-kaiti.png"))
		if err != nil {
			t.Fatal(err)
		}
		if err := png.Encode(f, tt.Image()); err != nil {
			t.Fatal(err)
		}
		f.Close()
	}
	for _, dark := range []bool{false, true} {
		for _, pg := range pages {
			s, _ := newTestSettings()
			s.page = pg.ID
			if pg.ID == "learned" {
				s.loadWords()
			}
			tt := ui.NewTester(s.view, 1400, 720)
			tt.SetDark(dark)
			tt.Frame()
			name := pg.ID
			if dark {
				name += "-dark"
			}
			f, err := os.Create(filepath.Join(dir, name+".png"))
			if err != nil {
				t.Fatal(err)
			}
			if err := png.Encode(f, tt.Image()); err != nil {
				t.Fatal(err)
			}
			f.Close()
		}
	}
}

func TestNewSettingsLayoutsSnapshots(t *testing.T) {
	directory := os.Getenv("FREETYPE_SNAPSHOT_DIR")
	if directory == "" {
		t.Skip("FREETYPE_SNAPSHOT_DIR 没设")
	}
	for _, sample := range []struct {
		name, target, query string
		width, height       int
	}{
		{"backup-min-window", "general/全部设置和数据", "", 780, 540},
		{"common-min-window", "common/导入或导出", "", 780, 540},
		{"search-hidden-option", "typing/汉字和字母之间", "", 1000, 540},
		{"mixed-original-style", "typing/中英文之间自动加空格", "", 1000, 600},
		{"about-min-window", "about/自动更新", "", 780, 540},
		{"after-update-min-window", "about/更新安装后显示设置页", "", 780, 540},
		{"about-updates-ready", "about/检查更新", "", 1000, 540},
		{"search-results", "", "中英 空格", 1000, 720},
	} {
		s, _ := newTestSettings()
		s.config.Autospace = false
		if sample.name == "mixed-original-style" {
			s.config.Autospace = true
		}
		s.common = []string{"您好，欢迎使用 FreeType。", "第一行\n第二行"}
		if sample.name == "about-updates-ready" {
			s.updateStatus = updateState{Version: "0.2.0", Message: "更新已下载，可立即更新；开启自动更新后，下次启动时安装", Ready: true}
		}
		s.onCheckUpdates, s.onUpdateNow = func() {}, func() {}
		if _, err := os.Stat("resources"); err == nil {
			s.resources = "resources"
		}
		if sample.target != "" {
			s.locateSetting(setting(sample.target))
		}
		s.settingQuery = sample.query
		tt := ui.NewTester(s.view, sample.width, sample.height)
		tt.Frame()
		file, err := os.Create(filepath.Join(directory, sample.name+".png"))
		if err != nil {
			t.Fatal(err)
		}
		if err := png.Encode(file, tt.Image()); err != nil {
			file.Close()
			t.Fatal(err)
		}
		file.Close()
	}
}

func TestMainLexiconCanBeChangedAndReset(t *testing.T) {
	s, b := newTestSettings()
	s.openTable = func(string) string { return `D:\dict\wanxiang\lexicon.tsv` }
	tt := open(t, s, "词库")
	if !tt.HasText("白霜＋万象＋项目热词（默认）") {
		t.Fatalf("没有显示默认词库：%q", tt.Texts())
	}
	if err := tt.Click("选择文件…"); err != nil {
		t.Fatal(err)
	}
	if b.config.Lexicon != `D:\dict\wanxiang\lexicon.tsv` {
		t.Fatalf("词库路径没有保存：%q", b.config.Lexicon)
	}
	tt.Frame()
	if err := tt.Click("恢复默认"); err != nil {
		t.Fatal(err)
	}
	if b.config.Lexicon != "" {
		t.Fatalf("没有恢复默认：%q", b.config.Lexicon)
	}
}

func TestTraySwitchDefaultsOnAndTellsTheTrayApp(t *testing.T) {
	s, b := newTestSettings()
	var shown []bool
	s.onTray = func(show bool) { shown = append(shown, show) }
	tt := open(t, s, "")
	if !b.config.TrayIcon {
		t.Fatal("托盘图标默认应开着")
	}
	if err := tt.Click("显示托盘图标"); err != nil {
		t.Fatal(err)
	}
	if b.config.TrayIcon || len(shown) != 1 || shown[0] {
		t.Fatalf("关掉托盘图标没生效：%+v %v", b.config.TrayIcon, shown)
	}
}
