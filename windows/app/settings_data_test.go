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

func TestRemoveLastQuickPhraseSendsAnEmptyProtocolArray(t *testing.T) {
	s, backend := newTestSettings()
	s.removePhrase(0)
	request := backend.last("set_quick_phrases")
	raw, err := json.Marshal(request["phrases"])
	if err != nil || string(raw) != "[]" {
		t.Fatalf("clearing quick phrases must send an empty array accepted by the Rust service: %s %v", raw, err)
	}
	if len(s.phrases) != 0 {
		t.Fatal("the removed phrase is still displayed")
	}
}

func TestDictionaryImportShowsReport(t *testing.T) {
	s, b := newTestSettings()
	s.openFile = func(string) string { return `C:\dl\常用词.txt` }
	tt := open(t, s, "词库")
	if err := tt.Click("导入词库文件"); err != nil {
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
	if err := tt.Click("导入"); err != nil {
		t.Fatal(err)
	}
	if b.last("import_common_phrases")["path"] != `C:\test\common.json` || !tt.HasText("导入常用语") {
		t.Fatalf("应导入并刷新常用语：%v %q", b.calls, tt.Texts())
	}
	if err := tt.Click("导出"); err != nil {
		t.Fatal(err)
	}
	if b.last("export_common_phrases")["path"] != `C:\test\export.json` {
		t.Fatal("未导出常用语")
	}
}

func TestFullSettingsImportNeedsConfirmationAndRefreshesAllData(t *testing.T) {
	s, b := newTestSettings()
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.WebDAVUser, s.sync.WebDAVPassword = "existing-account", "existing-password"
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	previous := s.sync
	backup := filepath.Join(t.TempDir(), "settings.json")
	if err := os.WriteFile(backup, []byte(`{"format":"freetype-settings","version":1}`), 0600); err != nil {
		t.Fatal(err)
	}
	s.openFile = func(title string) string {
		if title != "导入全部设置" {
			t.Fatalf("导入标题：%s", title)
		}
		return backup
	}
	var tray bool = true
	s.onTray = func(show bool) { tray = show }
	tt := open(t, s, "数据与同步")
	if err := tt.Click("导入全部设置"); err != nil {
		t.Fatal(err)
	}
	if b.last("inspect_settings_backup") == nil || b.last("import_settings") != nil {
		t.Fatal("确认前只能校验备份")
	}
	if err := tt.Click("导入并替换"); err != nil {
		t.Fatal(err)
	}
	if b.last("import_settings")["path"] != backup {
		t.Fatal("导入路径错误")
	}
	if s.config.PageSize != 9 || tray || s.dicts[0].Name != "恢复词库" || s.phrases[0].Text != "恢复短语" || s.common[0] != "恢复常用语" || s.words[0].Word != "恢复词" {
		t.Fatalf("导入后必须刷新全部数据与托盘开关：%+v", s)
	}
	if saved, err := loadSyncSettings(s.syncDir); err != nil || saved != previous {
		t.Fatal("旧备份不得改变同步账号和密码", err)
	}
}

func TestFullSettingsImportCancelAndInvalidFilePreserveCurrentData(t *testing.T) {
	for _, invalid := range []bool{false, true} {
		s, b := newTestSettings()
		backup := filepath.Join(t.TempDir(), "settings.json")
		if err := os.WriteFile(backup, []byte(`{"format":"freetype-settings","version":1}`), 0600); err != nil {
			t.Fatal(err)
		}
		s.openFile = func(string) string { return backup }
		if invalid {
			b.failRequest = "inspect_settings_backup"
		}
		tt := open(t, s, "数据与同步")
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
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.WebDAVUser, s.sync.WebDAVPassword = "test-account", "test-password-not-exported"
	s.sync.S3AccessKey, s.sync.S3SecretKey = "test-key", "test-secret-not-exported"
	s.sync.Automatic, s.sync.Schedule, s.sync.DailyTime = true, "daily", "22:15"
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	backup := filepath.Join(t.TempDir(), "all.json")
	s.saveFile = func(title, name string) string {
		if title != "导出全部设置" || name != "FreeType 全部设置.json" {
			t.Fatalf("导出信息错误：%s %s", title, name)
		}
		return backup
	}
	tt := open(t, s, "数据与同步")
	if err := tt.Click("导出全部设置"); err != nil {
		t.Fatal(err)
	}
	if b.last("export_settings") != nil || !tt.HasText("选择要导出的数据") {
		t.Fatal("点击导出后应先弹出数据类别选择框")
	}
	if err := tt.Click("导出选中的数据"); err != nil {
		t.Fatal(err)
	}
	if b.last("export_settings") == nil {
		t.Fatal("必须导出完整备份")
	}
	data, err := os.ReadFile(backup)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"test-account", "test-key", "22:15"} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("缺少账号或计划 %s", want)
		}
	}
	for _, secret := range []string{"test-password-not-exported", "test-secret-not-exported", "webdav_password", "s3_secret_key", s.sync.DeviceID} {
		if strings.Contains(string(data), secret) {
			t.Fatal("导出泄漏了密码、密钥或设备身份")
		}
	}
	// 在另一台电脑导入：恢复账号和计划，清除旧密码，保留目标设备身份并暂停同步。
	// 即使文件被加入凭据或设备字段，也不能从备份导入它们。
	var edited map[string]json.RawMessage
	if err := json.Unmarshal(data, &edited); err != nil {
		t.Fatal(err)
	}
	var connection map[string]any
	if err := json.Unmarshal(edited["sync_connection"], &connection); err != nil {
		t.Fatal(err)
	}
	connection["webdav_password"], connection["s3_secret_key"] = "injected-password", "injected-secret"
	connection["device_id"], connection["enabled"] = deviceA, true
	edited["sync_connection"], _ = json.Marshal(connection)
	data, _ = json.Marshal(edited)
	if err := os.WriteFile(backup, data, 0600); err != nil {
		t.Fatal(err)
	}
	target, _ := newTestSettings()
	target.syncDir = t.TempDir()
	target.loadSync()
	target.sync.WebDAVPassword, target.sync.S3SecretKey, target.sync.Enabled = "old-password", "old-secret", true
	if !target.saveSync() {
		t.Fatal(target.syncMessage)
	}
	id := target.sync.DeviceID
	target.importSettingsBackup(backup)
	saved, err := loadSyncSettings(target.syncDir)
	if err != nil || saved.WebDAVUser != "test-account" || saved.S3AccessKey != "test-key" || saved.DailyTime != "22:15" || !saved.Automatic || saved.Enabled || saved.WebDAVPassword != "" || saved.S3SecretKey != "" || saved.DeviceID != id {
		t.Fatalf("导入账号、计划、凭据隔离失败：%v", err)
	}
	// 无效计划和服务拒绝导入时，都不能先改连接配置。
	for _, fail := range []string{"invalid-plan", "import_settings"} {
		connection["daily_time"] = "22:15"
		if fail == "invalid-plan" {
			connection["daily_time"] = "invalid"
		}
		edited["sync_connection"], _ = json.Marshal(connection)
		data, _ = json.Marshal(edited)
		if err := os.WriteFile(backup, data, 0600); err != nil {
			t.Fatal(err)
		}
		fresh, backend := newTestSettings()
		fresh.syncDir = t.TempDir()
		fresh.loadSync()
		if !fresh.saveSync() {
			t.Fatal(fresh.syncMessage)
		}
		original := fresh.sync
		backend.failRequest = fail
		fresh.importSettingsBackup(backup)
		if got, err := loadSyncSettings(fresh.syncDir); err != nil || got != original || !strings.HasPrefix(fresh.notice, "导入失败") {
			t.Fatal("失败的导入不应改变连接设置", err)
		}
		if fail == "invalid-plan" && backend.last("import_settings") != nil {
			t.Fatal("无效同步配置必须在导入数据前拒绝")
		}
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
	if !tt.HasText("设置业") || !tt.HasText("学到的词 2 条") {
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
	if err := tt.Click("清空全部学习记录"); err != nil {
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
	if err := tester.Click("导出"); err != nil {
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
	if !tester.HasText("已导出 2 条；包含词条、选词次数和搭配") {
		t.Fatalf("导出结果应说明备份包含输入习惯：%q", tester.Texts())
	}
}

func TestLearningBackupExportCancelDoesNotCallServer(t *testing.T) {
	s, backend := newTestSettings()
	s.saveFile = func(string, string) string { return "" }
	tester := open(t, s, "学到的词")
	if err := tester.Click("导出"); err != nil {
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
	if err := tester.Click("导入"); err != nil {
		t.Fatal(err)
	}
	request := backend.last("import_learning")
	if request == nil || request["path"] != "C:/backup/learning.json" {
		t.Fatalf("没有请求导入完整学习记录：%v", request)
	}
	tester.Frame()
	if !tester.HasText("恢复词") || !tester.HasText("学到的词 3 条") {
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
	if err := tester.Click("导入"); err != nil {
		t.Fatal(err)
	}
	if backend.last("import_learning") != nil {
		t.Fatal("取消文件选择后不应执行导入")
	}
}

func TestLearningBackupErrorsPreserveWords(t *testing.T) {
	for _, test := range []struct{ operation, button string }{
		{"export_learning", "导出"},
		{"import_learning", "导入"},
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
	for _, label := range []string{"导入", "导出", "清空全部学习记录"} {
		bounds, found := tester.Find(label)
		if !found || bounds.X < 0 || bounds.X+bounds.W > 780 || bounds.Y+bounds.H > 540 {
			t.Fatalf("最小窗口中应能看到并使用 %s：%+v", label, bounds)
		}
	}
	count := 0
	for _, label := range tester.Texts() {
		if label == "导出" {
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

func TestMainLexiconCanBeChangedAndReset(t *testing.T) {
	s, b := newTestSettings()
	s.openTable = func(string) string { return `D:\dict\wanxiang\lexicon.tsv` }
	tt := open(t, s, "词库")
	if !tt.HasText("默认；白霜＋万象＋项目热词") {
		t.Fatalf("没有显示默认词库：%q", tt.Texts())
	}
	if err := tt.Click("选择文件"); err != nil {
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

func TestLexiconRecoveryIsVisibleWithoutReportingStartupFailure(t *testing.T) {
	s, _ := newTestSettings()
	const warning = "已使用默认词库；自定义词库第 3 行格式不正确"
	if err := json.Unmarshal([]byte(`{"ready":true,"lexicon_warning":"`+warning+`"}`), &s.status); err != nil {
		t.Fatal(err)
	}
	for _, page := range []string{"常规", "词库"} {
		view := open(t, s, page)
		if !view.HasText(warning) || view.HasText("词库载入失败："+warning) {
			t.Fatalf("%s 没有区分已恢复和载入失败：%v", page, view.Texts())
		}
	}
}

func TestDictionaryPageOffersSourceBrowsingAndTemporaryDisable(t *testing.T) {
	s, b := newTestSettings()
	b.dicts = []Dictionary{{Name: "专业词库", Entries: 12}}
	s.refresh()
	tt := open(t, s, "词库")
	tt.Frame()
	for _, label := range []string{"浏览 专业词库", "启用 专业词库"} {
		if err := tt.Click(label); err != nil {
			t.Fatalf("词库管理入口不可用：%s：%v", label, err)
		}
		tt.Frame()
	}
	if !b.dicts[0].Enabled || s.dictionaryPage == nil || s.dictionaryPage.Total != 12 {
		t.Fatal("启用状态与词库总数应回显")
	}
}

func TestPersonalWordsAndPhrasesCanBeDeletedAsSelectedGroups(t *testing.T) {
	s, b := newTestSettings()
	tt := open(t, s, "学到的词")
	for _, label := range []string{"选择学习词 设置业", "删除选中学习词", "确认删除学习词"} {
		if err := tt.Click(label); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
	}
	if len(b.words) != 1 || b.words[0].Word != "输入法" {
		t.Fatalf("只能删除所选词：%+v", b.words)
	}
	if err := tt.Click("快捷短语"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	for _, label := range []string{"选择短语 dz 1", "删除选中短语", "确认删除短语"} {
		if err := tt.Click(label); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
	}
	if len(b.phrases) != 0 {
		t.Fatal("选中短语没有删除")
	}
}

func TestSelectiveSettingsExportAllowsPickingDataAndBlocksOldServer(t *testing.T) {
	s, b := newTestSettings()
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.WebDAVUser, s.sync.Encrypt = "test-account", true
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	backup := filepath.Join(t.TempDir(), "partial.json")
	s.saveFile = func(string, string) string { return backup }
	tt := open(t, s, "数据与同步")
	if err := tt.Click("导出全部设置"); err != nil {
		t.Fatal(err)
	}
	// 只选「设置」和「快捷短语」，不导出词库、常用语、学习记录和同步账号
	for _, label := range []string{"导出词库", "导出常用语", "导出学习记录", "导出同步账号和计划"} {
		if err := tt.Click(label); err != nil {
			t.Fatalf("切换 %s 失败：%v", label, err)
		}
	}
	if err := tt.Click("导出选中的数据"); err != nil {
		t.Fatal(err)
	}
	req := b.last("export_settings")
	rawParts, _ := json.Marshal(req["parts"])
	if string(rawParts) != `["settings","quick_phrases"]` {
		t.Fatalf("应只向服务请求选中的类别：%s", rawParts)
	}
	data, err := os.ReadFile(backup)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(data), "sync_connection") || strings.Contains(string(data), "test-account") {
		t.Fatalf("未勾选同步账号时不应写入连接信息：%s", data)
	}
	if !strings.Contains(strings.Join(tt.Texts(), " "), "已导出选中的数据：") {
		t.Fatalf("部分导出提示不对：%q", tt.Texts())
	}
	// 只选「同步账号和计划」时，发给服务进程的 parts 必须是空数组 `[]` 而不是 null
	s.exportParts = []bool{false, false, false, false, false, true}
	if _, err := s.exportSettingsBackup(backup, s.chosenExportParts(), ""); err != nil {
		t.Fatal(err)
	}
	rawParts, _ = json.Marshal(b.last("export_settings")["parts"])
	if string(rawParts) != `[]` {
		t.Fatalf("只导出同步账号时应向服务传空数组：%s", rawParts)
	}
	data, _ = os.ReadFile(backup)
	if !strings.Contains(string(data), `"sync_connection"`) || !strings.Contains(string(data), `"encrypt": true`) {
		t.Fatalf("应写出同步账号与加密开关：%s", data)
	}
}

func TestEncryptedLocalBackupExportAndUnlockOnImport(t *testing.T) {
	s, b := newTestSettings()
	s.syncDir = t.TempDir()
	s.loadSync()
	s.sync.WebDAVUser, s.sync.Automatic, s.sync.Schedule, s.sync.DailyTime = "enc-account", true, "daily", "21:30"
	if !s.saveSync() {
		t.Fatal(s.syncMessage)
	}
	dir := t.TempDir()
	plainBackup := filepath.Join(dir, "plain.json")
	if _, err := s.exportSettingsBackup(plainBackup, s.chosenExportParts(), ""); err != nil {
		t.Fatal(err)
	}
	plainBytes, err := os.ReadFile(plainBackup)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(plainBytes), "\n  \"learning\": {\"format\":\"freetype-learning\",\"version\":1,\"words\":[[\"si mi\",\"私密词\",1000]]}") {
		t.Fatalf("未加密导出的学习记录也应压缩在单行内：%s", plainBytes)
	}

	encBackup := filepath.Join(dir, "encrypted.json")
	s.saveFile = func(string, string) string { return encBackup }
	tt := open(t, s, "数据与同步")
	if err := tt.Click("导出全部设置"); err != nil {
		t.Fatal(err)
	}
	s.exportPassword = "local-backup-password"
	if err := tt.Click("导出选中的数据"); err != nil {
		t.Fatal(err)
	}
	if s.exportPassword != "" {
		t.Fatal("导出后应立即清空内存中的导出密码")
	}
	encBytes, err := os.ReadFile(encBackup)
	if err != nil {
		t.Fatal(err)
	}
	if !isEncryptedBackup(encBytes) {
		t.Fatalf("设置了导出密码时必须生成加密备份：%s", encBytes)
	}
	for _, secret := range []string{"私密词", "enc-account", "freetype-settings"} {
		if strings.Contains(string(encBytes), secret) {
			t.Fatalf("加密备份不应出现明文 %s：%s", secret, encBytes)
		}
	}
	if !strings.Contains(strings.Join(tt.Texts(), " "), "已导出全部设置和数据；已加密") {
		t.Fatalf("加密导出提示不对：%q", tt.Texts())
	}

	// 导入加密备份：先弹出密码输入框，错误密码报错且不调用服务导入，正确密码解锁并恢复
	target, tb := newTestSettings()
	target.syncDir = t.TempDir()
	target.openFile = func(string) string { return encBackup }
	tt2 := open(t, target, "数据与同步")
	if err := tt2.Click("导入全部设置"); err != nil {
		t.Fatal(err)
	}
	if !target.unlockingRestore || tb.last("inspect_settings_backup") != nil {
		t.Fatal("选择加密备份后应先提示输入密码，不能直接交给服务校验")
	}
	target.restorePassword = "wrong-password"
	if err := tt2.Click("解锁备份"); err != nil {
		t.Fatal(err)
	}
	if !target.unlockingRestore || !strings.Contains(target.unlockError, "备份密码不正确") || tb.last("inspect_settings_backup") != nil {
		t.Fatalf("错误密码应留在解锁弹窗并提示错误：%q", target.unlockError)
	}
	target.restorePassword = "local-backup-password"
	if err := tt2.Click("解锁备份"); err != nil {
		t.Fatal(err)
	}
	if target.unlockingRestore || !target.confirmRestore || tb.last("inspect_settings_backup") == nil {
		t.Fatalf("正确密码解锁后应进入确认恢复弹窗：unlocking=%v confirm=%v err=%q", target.unlockingRestore, target.confirmRestore, target.unlockError)
	}
	if err := tt2.Click("导入并替换"); err != nil {
		t.Fatal(err)
	}
	if tb.last("import_settings") == nil || target.config.PageSize != 9 {
		t.Fatalf("确认后应解密并导入设置：%+v", target.config)
	}
	restoredSync, err := loadSyncSettings(target.syncDir)
	if err != nil || restoredSync.WebDAVUser != "enc-account" || restoredSync.DailyTime != "21:30" {
		t.Fatalf("加密备份中的同步账号和计划也应恢复：%+v %v", restoredSync, err)
	}
	_ = b
}

func TestDictionaryNameDropsRimeSuffix(t *testing.T) {
	for path, want := range map[string]string{
		`C:\rime\rime_ice.dict.yaml`: "rime_ice",
		`C:\rime\plain.yml`:          "plain",
		`C:\词库\我的词库.txt`:             "我的词库",
		`C:\词库\a.dict.txt`:           "a.dict",
	} {
		if got := dictionaryName(path); got != want {
			t.Errorf("dictionaryName(%q) = %q, want %q", path, got, want)
		}
	}
}
