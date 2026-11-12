package main

import (
	"encoding/json"
	"testing"

	"github.com/egoist/mygo/ui"
)

// fakeBackend 记下请求，按 op 给应答。
type fakeBackend struct {
	calls   []Request
	config  Config
	phrases []QuickPhrase
	dicts   []Dictionary
}

func (f *fakeBackend) Call(r Request) (Response, error) {
	f.calls = append(f.calls, r)
	switch r["op"] {
	case "status":
		return Response{OK: true, Status: &Status{Ready: true, Mode: "chinese", DataDir: `C:\data`}}, nil
	case "get_config":
		c := f.config
		return Response{OK: true, Config: &c}, nil
	case "set_config":
		// 走一遍 JSON，和真的管道一样
		raw, _ := json.Marshal(r["config"])
		_ = json.Unmarshal(raw, &f.config)
		c := f.config
		return Response{OK: true, Config: &c}, nil
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
	b := &fakeBackend{config: Config{PageSize: 7, Horizontal: true, FontSize: 18, ShuangpinScheme: "xiaohe", QuickPhrases: true}}
	s := newSettings(b, "0.0.1")
	s.inline = true
	s.refresh()
	return s, b
}

func TestGeneralPageSavesSwitches(t *testing.T) {
	s, b := newTestSettings()
	tt := ui.NewTester(s.view, 800, 900)
	if !tt.HasText("输入法服务运行中，当前为中文状态") {
		t.Fatalf("没有显示服务状态：%q", tt.Texts())
	}
	if err := tt.Click("模糊音"); err != nil {
		t.Fatal(err)
	}
	if b.last("set_config") == nil || !b.config.Fuzzy {
		t.Fatalf("开关没有保存：%+v", b.config)
	}
	if b.config.PageSize != 7 || b.config.ShuangpinScheme != "xiaohe" {
		t.Fatalf("其他设置被改了：%+v", b.config)
	}
}

func TestQuickPhraseAddValidatesAndSaves(t *testing.T) {
	s, b := newTestSettings()
	s.page = "phrases"
	tt := ui.NewTester(s.view, 800, 900)
	s.newCode, s.newText = "d1", "地址"
	if err := tt.Click("添加"); err != nil {
		t.Fatal(err)
	}
	if s.formError == "" || b.last("set_quick_phrases") != nil {
		t.Fatal("编码带数字应该报错、不保存")
	}
	s.newCode, s.newText, s.newWeight = "DZ", "四川省\r\n宜宾市", "20"
	if err := tt.Click("添加"); err != nil {
		t.Fatal(err)
	}
	if len(b.phrases) != 1 || b.phrases[0] != (QuickPhrase{Code: "dz", Text: "四川省\n宜宾市", Weight: 20}) {
		t.Fatalf("保存的短语不对：%+v", b.phrases)
	}
	tt.Advance(0)
	if !tt.HasText("四川省 ↵") {
		t.Fatalf("列表里应只显示第一行：%q", tt.Texts())
	}
	if err := tt.Click("删除 dz 1"); err != nil {
		t.Fatal(err)
	}
	if len(b.phrases) != 0 {
		t.Fatalf("没有删掉：%+v", b.phrases)
	}
}

func TestDictionaryImportShowsReport(t *testing.T) {
	s, b := newTestSettings()
	s.page = "dicts"
	s.openFile = func(string) string { return `C:\词库\人名.txt` }
	tt := ui.NewTester(s.view, 800, 900)
	if err := tt.Click("导入词库…"); err != nil {
		t.Fatal(err)
	}
	if r := b.last("import_dictionary"); r == nil || r["name"] != "人名" {
		t.Fatalf("导入请求不对：%v", r)
	}
	tt.Advance(0)
	for _, want := range []string{"「人名」导入 2 条，跳过 1 行", "第 3 行：编码不是有效的全拼", "人名", "2 条"} {
		if !tt.HasText(want) {
			t.Fatalf("缺少 %q：%q", want, tt.Texts())
		}
	}
}

func TestSchemeNamesRoundTrip(t *testing.T) {
	for _, sc := range schemes {
		if schemeID(schemeName(sc.ID)) != sc.ID {
			t.Fatalf("%s 对不上", sc.ID)
		}
	}
	if schemeName("unknown") != "小鹤双拼" {
		t.Fatal("未知方案应退回小鹤")
	}
}
