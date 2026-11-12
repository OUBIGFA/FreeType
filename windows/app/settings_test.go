package main

import (
	"encoding/json"
	"fmt"
	"image/png"
	"os"
	"path/filepath"
	"testing"

	"github.com/egoist/mygo/ui"
)

// fakeBackend 记下请求，按 op 给应答。
type fakeBackend struct {
	calls   []Request
	config  Config
	phrases []QuickPhrase
	dicts   []Dictionary
	words   []UserWord
	mode    string
}

func (f *fakeBackend) Call(r Request) (Response, error) {
	f.calls = append(f.calls, r)
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
	case "list_dictionaries":
		return Response{OK: true, Dictionaries: f.dicts}, nil
	case "import_dictionary":
		f.dicts = append(f.dicts, Dictionary{Name: r["name"].(string), Entries: 2})
		return Response{OK: true, Report: &ImportReport{Added: 2, Skipped: 1, Errors: []string{"第 3 行：编码不是有效的全拼"}}}, nil
	case "list_user_words":
		return Response{OK: true, UserWords: f.words}, nil
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
	if !tt.HasText("每页 6 个") || !tt.HasText("妮") || tt.HasText("呢") {
		t.Fatalf("预览没按每页 6 个画：%q", tt.Texts())
	}
	s.config.PageSize, s.config.FontFamily = 8, "SimSun"
	tt.Frame()
	if !tt.HasText("呢") || !tt.HasText("预览 · SimSun") {
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
	fontNames = func() []string { return []string{"Microsoft YaHei", "SimSun"} }
	for _, dark := range []bool{false, true} {
		for _, pg := range pages {
			s, _ := newTestSettings()
			s.page = pg.ID
			if pg.ID == "learned" {
				s.loadWords()
			}
			tt := ui.NewTester(s.view, 1000, 720)
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

func TestMainLexiconCanBeChangedAndReset(t *testing.T) {
	s, b := newTestSettings()
	s.openTable = func(string) string { return `D:\dict\wanxiang\lexicon.tsv` }
	tt := open(t, s, "词库")
	if !tt.HasText("雾凇拼音（随安装包，默认）") {
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
