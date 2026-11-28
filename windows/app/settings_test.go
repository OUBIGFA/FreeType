package main

import (
	"encoding/json"
	"fmt"
	"os"
	"reflect"
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
		// 服务端对缺少的字段补默认值，空对象用于恢复出厂设置。
		f.config = defaultConfig()
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
	case "list_apps":
		return Response{OK: true, Apps: []AppWindow{{App: "code.exe", Title: "main.go"}, {App: "wechat.exe", Title: "微信"}}}, nil
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
	case "query_dictionary":
		for _, dict := range f.dicts {
			if dict.Name == r["name"] {
				return Response{OK: true, DictionaryPage: &DictionaryPage{Dictionary: dict, Total: dict.Entries}}, nil
			}
		}
		return Response{}, fmt.Errorf("找不到词库")
	case "update_dictionary":
		for i := range f.dicts {
			if f.dicts[i].Name == r["name"] {
				f.dicts[i].Name, f.dicts[i].Enabled, f.dicts[i].Description = r["new_name"].(string), r["enabled"].(bool), r["description"].(string)
			}
		}
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
	case "export_settings", "export_sync_settings":
		rawParts, _ := json.Marshal(r["parts"])
		content := `{"format":"freetype-settings","version":1}`
		if r["parts"] == nil || strings.Contains(string(rawParts), `"learning"`) {
			content = "{\n  \"format\": \"freetype-settings\",\n  \"version\": 1,\n  \"learning\": {\n    \"format\": \"freetype-learning\",\n    \"version\": 1,\n    \"words\": [\n      [\n        \"si mi\",\n        \"私密词\",\n        1000\n      ]\n    ]\n  }\n}\n"
		}
		if err := os.WriteFile(r["path"].(string), []byte(content), 0600); err != nil {
			return Response{}, err
		}
		return Response{OK: true, Report: &ImportReport{Added: 5}, ModifiedUnixMS: 100, SyncFormat: 1}, nil
	case "preview_sync_merge":
		return Response{OK: true, SyncMerge: &SyncMergePreview{Added: 2, Deleted: 1, Conflicts: 1, Versions: 3, Details: []string{"删除 常用语：旧条目"}}}, nil
	case "merge_sync_settings":
		return Response{OK: true, SyncMerge: &SyncMergePreview{Added: 2, Deleted: 1, Versions: 3}}, nil
	case "reset_sync_records":
		return Response{OK: true}, nil
	case "import_sync_settings":
		f.config.PageSize = 9
		f.common = []string{"按正文恢复的常用语"}
		c := f.config
		return Response{OK: true, Config: &c, Status: &Status{Ready: true, Mode: "chinese"}, CommonPhrases: f.common}, nil
	case "inspect_settings_backup":
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
	// 窗口足够高，整页设置无需滚动即可点到；设计改动让行变高时同步调这里。
	tt := ui.NewTester(s.view, 1000, 2400)
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

func TestDefaultInputMethodPrecedesStatusAndUsesGeneralSectionBackground(t *testing.T) {
	for _, dark := range []bool{false, true} {
		t.Run(fmt.Sprintf("dark=%t", dark), func(t *testing.T) {
			s, _ := newTestSettings()
			view := ui.NewTester(s.view, 780, 720)
			view.SetDark(dark)
			view.Frame()
			serviceStatus, hasServiceStatus := view.Find("FreeType 输入法")
			defaultTip, hasDefaultTip := view.Find("默认输入法")
			status, hasStatus := view.Find("现在的状态")
			if !hasServiceStatus || !hasDefaultTip || !hasStatus || serviceStatus.Y+serviceStatus.H > defaultTip.Y || defaultTip.Y+defaultTip.H > status.Y {
				t.Fatal("常规页应依次显示服务状态卡、默认输入法和现在的状态")
			}
			image := view.Image()
			rowBackground := func(label string) [4]uint8 {
				t.Helper()
				bounds, ok := view.Find(label)
				if !ok {
					t.Fatalf("常规页缺少 %q", label)
				}
				// Sample the left padding, clear of the text and the card border.
				pixel := image.RGBAAt(int(bounds.X)-8, int(bounds.Y+bounds.H/2))
				return [4]uint8{pixel.R, pixel.G, pixel.B, pixel.A}
			}
			if got, want := rowBackground("默认输入法"), rowBackground("简体 / 繁体"); got != want {
				t.Fatalf("默认输入法应与常规设置区块使用同一底色：%v，预期 %v", got, want)
			}
		})
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
	if err := tt.Click("an = ai"); err != nil {
		t.Fatal(err)
	}
	if err := tt.Click("eng = ong"); err != nil {
		t.Fatal(err)
	}
	if b.config.FuzzyGroups&4096 == 0 || b.config.FuzzyGroups&32768 == 0 {
		t.Fatalf("新增模糊音分组没有保存：%+v", b.config)
	}
	if err := tt.Click("双拼"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.Click("双拼展示键位"); err != nil {
		t.Fatal(err)
	}
	if !b.config.Shuangpin || !b.config.ShuangpinShowKeys {
		t.Fatalf("双拼展示键位没有保存：%+v", b.config)
	}
	for _, control := range tt.AccessAll() {
		if control.Name == "左右方向键" && control.Focusable {
			r := control.Bounds
			tt.ClickAt(r.X+r.W/2, r.Y+r.H/2)
			if err := tt.Click("移动拼音光标"); err != nil {
				t.Fatal(err)
			}
			break
		}
	}
	if b.config.ArrowKeys != "caret" {
		t.Fatalf("左右方向键设置没有保存：%+v", b.config)
	}
	if b.config.AutoPairAbsorb {
		t.Fatal("连打两半时不重复右标点应默认关闭")
	}
	tt.Frame()
	if err := tt.Click("连打两半时不重复右标点"); err != nil {
		t.Fatal(err)
	}
	if !b.config.AutoPairAbsorb {
		t.Fatal("连打两半时不重复右标点没有保存")
	}
	if err := tt.Click("成对标点自动补全"); err != nil {
		t.Fatal(err)
	}
	if b.config.AutoPair {
		t.Fatal("成对标点没有关掉")
	}
}

func TestTypingPagePunctuationOptions(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "拼音与按键")
	if !b.config.FullwidthPunct || !b.config.DigitHalfwidthPunct || !b.config.DigitPunctExceptions || b.config.DigitPunctFullwidth != digitPunctDefault || b.config.CornerQuotes {
		t.Fatalf("标点默认配置不对：%+v", b.config)
	}
	for _, text := range []string{"数字标点保持半角", "数字后例外保持全角", "、 顿号", "： 冒号", "双引号改用直角引号"} {
		if !tt.HasText(text) {
			t.Fatalf("没有显示「%s」：%q", text, tt.Texts())
		}
	}
	// 勾选冒号作为数字后全角例外，开启双引号改用直角引号
	if err := tt.Click("： 冒号"); err != nil {
		t.Fatal(err)
	}
	if b.config.DigitPunctFullwidth&(1<<0) == 0 || b.config.DigitPunctFullwidth&(1<<1) == 0 {
		t.Fatalf("数字后全角标点勾选没有保存：%+v", b.config)
	}
	if err := tt.Click("双引号改用直角引号"); err != nil {
		t.Fatal(err)
	}
	if !b.config.CornerQuotes {
		t.Fatalf("双引号改用直角引号没有保存：%+v", b.config)
	}
	// 关掉例外选项后隐藏符号勾选框；关掉数字标点保持半角后隐藏例外选项；关掉中文标点后隐藏子项
	if err := tt.Click("数字后例外保持全角"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.config.DigitPunctExceptions || tt.HasText("、 顿号") {
		t.Fatalf("关掉数字后例外保持全角后不应再显示符号勾选：%+v", b.config)
	}
	if err := tt.Click("数字标点保持半角"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.config.DigitHalfwidthPunct || tt.HasText("数字后例外保持全角") {
		t.Fatalf("关掉数字标点保持半角后不应再显示例外开关：%+v", b.config)
	}
	if err := tt.Click("中文标点"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.config.FullwidthPunct || tt.HasText("数字标点保持半角") {
		t.Fatalf("关掉中文标点后不应再显示数字标点子项：%+v", b.config)
	}
}

func TestCandidatesPagePreviewsFont(t *testing.T) {
	fontNames = func() []string { return []string{"SimSun", "Arial", "Microsoft YaHei"} }
	s, _ := newTestSettings()
	tt := open(t, s, "候选窗与外观")
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

func TestCandidatesPageCustomSkinTemplatesToggleAndPreservation(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "候选窗与外观")
	if tt.HasText("选中底色块") || tt.HasText("窗口背景") {
		t.Fatal("内置样式下不应展开自定义皮肤配色面板")
	}
	s.config.CandidateStyle = "custom"
	s.saveConfig()
	tt.Frame()
	for _, text := range []string{"选中底色块", "窗口背景", "窗口边框", "候选文字", "拼音与序号", "选中背景", "选中文字", "霜白", "宣纸", "墨夜", "月影", "恢复默认配色"} {
		if !tt.HasText(text) {
			t.Fatalf("自定义皮肤面板缺少「%s」：%q", text, tt.Texts())
		}
	}
	if err := tt.Click("宣纸"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.config.CandidateSkin.Background != "#f9f6ef" || b.config.CandidateSkin.Accent != "#a9442a" {
		t.Fatalf("点击模板未保存对应配色：%+v", b.config.CandidateSkin)
	}
	if err := tt.Click("选中底色块"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.config.CandidateSkin.AccentFill || tt.HasText("选中背景") {
		t.Fatalf("关闭选中底色块后应隐藏选中背景色项：%+v", b.config.CandidateSkin)
	}
	s.config.CandidateStyle = "pink"
	s.saveConfig()
	tt.Frame()
	if b.config.CandidateSkin.Background != "#f9f6ef" {
		t.Fatalf("切回内置样式不应丢失自定义配色：%+v", b.config.CandidateSkin)
	}
	if results := findSettings("皮肤"); len(results) < 2 {
		t.Fatalf("搜索「皮肤」应定位候选皮肤与自定义配色：%v", results)
	}
}

func TestCandidateSkinTemplatesMeetContrastAndBalance(t *testing.T) {
	seen := map[string]bool{}
	lightCount, darkCount := 0, 0
	for _, tpl := range candidateSkinTemplates {
		if tpl.name == "" {
			t.Fatal("预设配色名称不能为空")
		}
		if seen[tpl.name] {
			t.Fatalf("预设配色名称重复：%s", tpl.name)
		}
		seen[tpl.name] = true

		colorsToCheck := []struct {
			label, raw string
		}{
			{"Background", tpl.skin.Background},
			{"Border", tpl.skin.Border},
			{"Text", tpl.skin.Text},
			{"Muted", tpl.skin.Muted},
			{"Accent", tpl.skin.Accent},
			{"AccentText", tpl.skin.AccentText},
		}
		for _, item := range colorsToCheck {
			if _, ok := parseHexRGB(item.raw); !ok {
				t.Fatalf("预设「%s」的 %s 颜色格式无效：%q", tpl.name, item.label, item.raw)
			}
		}

		bg, _ := parseHexRGB(tpl.skin.Background)
		text, _ := parseHexRGB(tpl.skin.Text)
		muted, _ := parseHexRGB(tpl.skin.Muted)
		accent, _ := parseHexRGB(tpl.skin.Accent)
		accentText, _ := parseHexRGB(tpl.skin.AccentText)

		if got := colorContrast(text, bg); got < 7.0 {
			t.Errorf("预设「%s」正文对比度不足：%.2f:1（要求 >= 7.0:1）", tpl.name, got)
		}
		if got := colorContrast(muted, bg); got < 4.5 {
			t.Errorf("预设「%s」拼音序号对比度不足：%.2f:1（要求 >= 4.5:1）", tpl.name, got)
		}
		selBg := bg
		if tpl.skin.AccentFill {
			selBg = accent
		}
		if got := colorContrast(accentText, selBg); got < 4.5 {
			t.Errorf("预设「%s」选中文字对比度不足：%.2f:1（要求 >= 4.5:1）", tpl.name, got)
		}

		lum := colorLuminance(bg)
		if lum > 0.7 {
			lightCount++
		} else if lum < 0.1 {
			darkCount++
		}
	}
	if lightCount < 4 || darkCount < 4 {
		t.Fatalf("浅色与深色预设数量应各不少于 4 套：light=%d dark=%d", lightCount, darkCount)
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

func TestLongWordCompletionSwitchSavesAndReloads(t *testing.T) {
	s, backend := newTestSettings()
	s.locateSetting(setting("features/长词前缀补全"))
	tt := ui.NewTester(s.view, 1000, 1400)
	tt.Frame()
	if !s.config.LongWordCompletion {
		t.Fatal("长词前缀补全应默认开启")
	}
	if err := tt.Click("词语前缀补全"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	s.refresh()
	tt.Frame()
	if s.config.LongWordCompletion || backend.config.LongWordCompletion {
		t.Fatal("关闭补全后，保存和重新读取必须保留选择")
	}
	if len(findSettings("长词")) == 0 {
		t.Fatal("搜索应能定位补全设置")
	}
}

func TestSentenceEditorDefaultsOffAndSavesIndependently(t *testing.T) {
	s, backend := newTestSettings()
	s.locateSetting(setting("candidates/整句编辑区"))
	tt := ui.NewTester(s.view, 1000, 1800)
	tt.Frame()
	if s.config.SentenceEditor {
		t.Fatal("整句编辑区应默认关闭")
	}
	before := s.config
	for _, enabled := range []bool{true, false} {
		if err := tt.Click("整句编辑区"); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
		s.refresh()
		tt.Frame()
		if s.config.SentenceEditor != enabled || backend.config.SentenceEditor != enabled {
			t.Fatal("整句编辑区开关没有保存")
		}
		got := s.config
		got.SentenceEditor = before.SentenceEditor
		if !reflect.DeepEqual(got, before) {
			t.Fatal("开关编辑区不应改变候选窗其他设置")
		}
	}
}

// 设了 FREETYPE_SNAPSHOT_DIR 时把每一页画成 PNG（看设计用，平时跳过）。

// 没开「记录剪贴板历史」时，指定软件的规则区也必须能操作（以前整块被禁用成灰色）。
