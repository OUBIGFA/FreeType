package main

import (
	"fmt"
	"image/png"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/egoist/mygo/ui"
)

// 离屏截图来自生产视图，不启动设置窗口或接触用户当前桌面。
func saveUIFrame(test *testing.T, name string, view *ui.Tester) {
	test.Helper()
	directory := os.Getenv("FREETYPE_SNAPSHOT_DIR")
	if directory == "" {
		return
	}
	file, err := os.Create(filepath.Join(directory, name+".png"))
	if err != nil {
		test.Fatal(err)
	}
	encodeErr := png.Encode(file, view.Image())
	closeErr := file.Close()
	if encodeErr != nil || closeErr != nil {
		test.Fatalf("截图失败：%v，关闭文件：%v", encodeErr, closeErr)
	}
}

func TestInputFeatureManagementViewsRenderRealControls(test *testing.T) {
	for _, dark := range []bool{false, true} {
		s, b := newTestSettings()
		b.dicts = []Dictionary{{Name: "专业词库", Entries: 120, Enabled: true, Source: "Rime · 医学.dict.yaml", Description: "日常医学用词"}}
		s.refresh()
		s.page, s.dictionaryName, s.dictionaryRename, s.dictionaryDescription = "dicts", "专业词库", "专业词库", "日常医学用词"
		s.dictionaryPage = &DictionaryPage{Dictionary: b.dicts[0], Total: 120, Entries: []DictionaryEntry{{Word: "研究", Pinyin: "yan jiu", Weight: 1000}, {Word: "临床试验", Pinyin: "lin chuang shi yan", Weight: 800}}}
		s.dictionarySelected = map[string]bool{}
		view := ui.NewTester(s.view, 1100, 1100)
		view.SetDark(dark)
		view.Frame()
		for _, label := range []string{"保存名称与说明", "搜索词库词条", "词库下一页", "删除选中词库词条"} {
			if _, found := view.Find(label); !found {
				test.Fatalf("管理控件缺失：%s", label)
			}
		}
		saveUIFrame(test, fmt.Sprintf("input-feature-dictionary-%t", dark), view)
		s.page = "features"
		s.locateSetting(setting("features/英文拼写建议"))
		view.Frame()
		view.Frame()
		if _, found := view.Find("英文拼写建议"); !found {
			test.Fatal("英文候选开关不可见")
		}
		saveUIFrame(test, fmt.Sprintf("input-feature-options-%t", dark), view)
		s.page = "sync"
		s.locatedSetting, s.settingQuery, s.revealSetting = "", "", false
		s.pageScroll = ui.ScrollState{}
		s.syncLoaded = true
		s.syncPreview = &SyncMergePreview{Added: 2, Updated: 1, Deleted: 1, Conflicts: 1, Details: []string{"删除 常用语：旧条目", "新增 学习：新用词"}}
		view = ui.NewTester(s.view, 1100, 2600)
		view.SetDark(dark)
		view.Frame()
		if !view.HasText("确认合并") || !view.HasText("整份替换本机数据") {
			test.Fatalf("同步预览缺少明确的恢复方式：%q", view.Texts())
		}
		saveUIFrame(test, fmt.Sprintf("input-feature-sync-%t", dark), view)
	}
}

func TestSettingsRowsShareInsetsAndTitleAlignment(test *testing.T) {
	checked := false
	view := ui.NewTester(func(context *ui.Context) {
		context.SetTheme(settingsTheme(context))
		ui.Column(context).FillWidth().Children(func() {
			ui.Column(context).Label("普通行").Children(func() { row(context, "普通标题", "", nil) })
			ui.Column(context).Label("开关行").Children(func() { switchLine(context, "开关标题", "", &checked, false) })
			ui.Column(context).Label("子级行").Children(func() { subRow(context, "子级标题", "", nil) })
			ui.Column(context).Label("子级开关行").Children(func() { switchLine(context, "子级开关标题", "", &checked, true) })
		})
	}, 640, 480)
	for _, pair := range [][4]string{
		{"普通行", "普通标题", "开关行", "开关标题"},
		{"子级行", "子级标题", "子级开关行", "子级开关标题"},
	} {
		first, _ := view.Find(pair[0])
		firstTitle, _ := view.Find(pair[1])
		second, _ := view.Find(pair[2])
		secondTitle, _ := view.Find(pair[3])
		if math.Abs(float64(first.H-second.H)) > 1 || math.Abs(float64(firstTitle.X-secondTitle.X)) > 1 || math.Abs(float64(firstTitle.Y-first.Y-secondTitle.Y+second.Y)) > 1 {
			test.Errorf("同级设置行必须共用高度、左边缘和标题基线：%+v / %+v，标题 %+v / %+v", first, second, firstTitle, secondTitle)
		}
	}
}

func TestSettingsSubLevelsIndentProgressively(test *testing.T) {
	state, _ := newTestSettings()
	state.page = "features"
	state.config.PredictNext, state.config.PredictSpace = true, true
	view := ui.NewTester(state.view, 1000, 1400)
	view.Frame()
	parent, ok1 := view.Find("上屏联想")
	child, ok2 := view.Find("空格选联想词")
	grandchild, ok3 := view.Find("双击空格键选词")
	if !ok1 || !ok2 || !ok3 {
		test.Fatalf("联想层级设置项缺失：parent=%v child=%v grandchild=%v", ok1, ok2, ok3)
	}
	if !(parent.X < child.X && child.X < grandchild.X) {
		test.Fatalf("一、二、三级设置标题应逐级缩进：parent=%.1f child=%.1f grandchild=%.1f", parent.X, child.X, grandchild.X)
	}
}

func TestCandidateSelectionPaletteMatchesStylesInEveryAppearance(test *testing.T) {
	for _, dark := range []bool{false, true} {
		theme := ui.LightTheme()
		if dark {
			theme = ui.DarkTheme()
		}
		for _, id := range []string{"pink", "blue", "green", "sogou"} {
			style := candidateStyle(id, dark)
			if id == "pink" {
				if !style.block || style.fill != brandPink || style.text != ui.Hex("#18181b") {
					test.Errorf("dark=%v：默认粉色预览必须使用 #FFB3FA 底深色字，得到 %+v", dark, style)
				}
			}
			if id == "green" {
				if !style.block || style.fill != ui.Hex("#03d468") || style.text != ui.Hex("#ffffff") {
					test.Errorf("dark=%v：微信绿预览必须使用 #03D468 绿底白字，得到 %+v", dark, style)
				}
			}
			background := theme.Background
			if style.block {
				background = style.fill
			}
			minimum := 4.5
			if id == "green" {
				// 保留鲜亮绿底白字，同时防止对比度退回原先较浅的绿色。
				minimum = 1.95
			}
			if contrast := colorContrast(style.text, background); contrast < minimum {
				test.Errorf("%s dark=%v：候选文字对比度 %.2f:1，需至少 %.1f:1", id, dark, contrast, minimum)
			}
		}
	}

	cfg := defaultConfig()
	cfg.CandidateStyle = "custom"
	cfg.CandidateSkin = CandidateSkin{
		Background: "#fbf8f1",
		Border:     "#e2d9c8",
		Text:       "#2c261f",
		Muted:      "#7a7062",
		Accent:     "#b45309",
		AccentText: "#ffffff",
		AccentFill: true,
	}
	ct := resolveCandidateTheme(cfg, false)
	if !ct.block || ct.background != ui.Hex("#fbf8f1") || ct.border != ui.Hex("#e2d9c8") || ct.accent != ui.Hex("#b45309") {
		test.Fatalf("自定义皮肤解析不正确：%+v", ct)
	}
	cfg.CandidateSkin.AccentFill = false
	ctNoFill := resolveCandidateTheme(cfg, true)
	if ctNoFill.block || ctNoFill.text != ui.Hex("#2c261f") {
		test.Fatalf("关闭底色块后应保留正文颜色并取消底色块：%+v", ctNoFill)
	}
}

func TestSettingsSearchResultsCanBeSelectedWithKeyboard(test *testing.T) {
	state, backend := newTestSettings()
	view := ui.NewTester(state.view, 780, 540)
	if err := view.Click("搜索全部设置"); err != nil {
		test.Fatal(err)
	}
	view.Type("自动加空格")
	view.Key(0, ui.KeyTab)
	view.Key(0, ui.KeyEnter)
	view.Frame()
	if state.page != "typing" || state.settingQuery != "" {
		test.Fatalf("Tab 和 Enter 应打开第一个搜索结果：page=%s query=%q", state.page, state.settingQuery)
	}
	if backend.last("set_config") != nil {
		test.Fatal("键盘定位设置不能替用户修改配置")
	}
}

func TestSettingsNavigationCanBeSelectedWithKeyboard(test *testing.T) {
	state, _ := newTestSettings()
	view := ui.NewTester(state.view, 1000, 900)
	view.Key(0, ui.KeyTab)
	view.Key(0, ui.KeyTab)
	view.Key(0, ui.KeyEnter)
	view.Frame()
	if state.page != "candidates" {
		test.Fatalf("侧边栏必须支持 Tab 和 Enter：page=%s", state.page)
	}
}

func TestCandidatePreviewCentersWhenItFits(test *testing.T) {
	state, _ := newTestSettings()
	state.page = "candidates"
	for _, horizontal := range []bool{true, false} {
		state.config.Horizontal = horizontal
		view := ui.NewTester(state.view, 1024, 900)
		stage, stageOK := view.Find("候选预览区域")
		popup, popupOK := view.Find("候选预览窗口")
		if !stageOK || !popupOK || popup.W >= stage.W || math.Abs(float64(2*(popup.X-stage.X)+popup.W-stage.W)) > 2 {
			test.Errorf("横排=%v：预览应在区域内居中：区域=%+v，预览=%+v", horizontal, stage, popup)
		}
	}
}

func TestCandidatePreviewTracksEditorPositionAndVisibleOptions(test *testing.T) {
	state, _ := newTestSettings()
	state.page = "candidates"
	view := ui.NewTester(state.view, 1100, 1000)
	if _, ok := view.Find("整句编辑区预览"); ok {
		test.Fatal("关闭时不应显示编辑区")
	}
	state.config.SentenceEditor = true
	for _, position := range []string{"below", "above"} {
		state.config.SentenceEditorPosition = position
		view.Frame()
		editor, ok := view.Find("整句编辑区预览")
		popup, _ := view.Find("候选预览窗口")
		if !ok || (position == "below" && editor.Y < popup.Y+popup.H) || (position == "above" && editor.Y+editor.H > popup.Y) {
			test.Fatalf("编辑区位置未同步：%s %+v %+v", position, editor, popup)
		}
		saveUIFrame(test, "candidate-editor-"+position, view)
	}
	state.config.PinBadge = "pushpin"
	state.config.InlinePreedit = "raw"
	state.config.CandidatePreedit = false
	view.Frame()
	if !view.HasText("📌") || !view.HasText("nihao│") || view.HasText("ni hao") {
		test.Fatalf("角标或预编辑未同步：%q", view.Texts())
	}
	state.config.PinBadge = "off"
	state.config.InlinePreedit = "off"
	view.Frame()
	if view.HasText("📌") || view.HasText("nihao│") {
		test.Fatal("关闭后仍显示预览")
	}
	state.config.CandidateFollow = false
	before, _ := view.Find("候选预览窗口")
	if err := view.Click("移动示例光标"); err != nil {
		test.Fatal(err)
	}
	view.Frame()
	after, _ := view.Find("候选预览窗口")
	if before.X != after.X {
		test.Fatal("不跟随光标时不应移动候选预览")
	}
	state.config.CandidateFollow = true
	view.Frame()
	after, _ = view.Find("候选预览窗口")
	if before.X == after.X {
		test.Fatal("开启跟随后应跟随示例光标")
	}
	state.config.FontSize, state.config.PageSize = 36, 10
	state.config.CandidateSingleLine = false
	view.Frame()
	wrapped, _ := view.Find("候选预览窗口")
	state.config.CandidateSingleLine = true
	view.Frame()
	single, _ := view.Find("候选预览窗口")
	if wrapped.H <= single.H {
		test.Fatalf("关闭单行后应换行：%+v %+v", wrapped, single)
	}
}

func TestCandidatePreviewKeepsLargeCandidatesReadableAndScrollable(test *testing.T) {
	state, _ := newTestSettings()
	state.page = "candidates"
	state.config.FontSize, state.config.PageSize, state.config.Horizontal = 36, 10, true
	view := ui.NewTester(state.view, 780, 720)
	view.Frame()
	saveUIFrame(test, "candidate-large-before-scroll", view)
	first, found := view.Find("你好")
	if !found || first.W < 60 || first.X < 236 || first.X+first.W > 756 {
		test.Errorf("大字号候选不能被挤窄或画出页面：%+v，存在=%v", first, found)
	}
	preedit, found := view.Find("ni hao")
	if !found {
		test.Fatal("预览必须显示预编辑文字")
	}
	before, found := view.Find("›")
	if !found {
		test.Fatal("预览必须包含下一页箭头")
	}
	view.Scroll(700, preedit.Y+preedit.H+16, 10000, 0)
	view.Frame()
	after, found := view.Find("›")
	if !found || after.X >= before.X || after.X < 236 || after.X+after.W > 756 {
		test.Errorf("横向滚动后应能看到最后的翻页箭头：before=%+v after=%+v", before, after)
	}
	saveUIFrame(test, "candidate-large-after-scroll", view)
}

func TestSettingsExplicitHeadingsFollowWindowsTextScale(test *testing.T) {
	state, _ := newTestSettings()
	view := ui.NewTester(state.view, 1000, 720)
	before, found := view.Find("FreeType 输入法")
	if !found {
		test.Fatal("没有输入法状态标题")
	}
	view.SetPreferences(ui.Preferences{TextScale: 1.5})
	after, found := view.Find("FreeType 输入法")
	if !found || after.H < before.H*1.3 {
		test.Fatalf("系统文字放大时，显式标题也应放大：before=%+v after=%+v", before, after)
	}
}

func TestSettingsHighContrastKeepsVisibleSurfaceBoundaries(test *testing.T) {
	for _, dark := range []bool{false, true} {
		var actual palette
		view := ui.NewTester(func(context *ui.Context) {
			context.SetTheme(settingsTheme(context))
			actual = colors(context)
		}, 780, 540)
		view.SetDark(dark)
		view.SetPreferences(ui.Preferences{HighContrast: true})
		if contrast := colorContrast(actual.border, actual.card); contrast < 3 {
			test.Errorf("高对比度下不能继续使用浅灰面板边界：dark=%v contrast=%.2f:1", dark, contrast)
		}
	}
}

func TestSettingsNarrowActionsStayInsideThePage(test *testing.T) {
	cases := []struct {
		page, target string
		controls     []string
	}{
		{"features", "features/指定软件规则", []string{"切入时", "添加 / 更新", "刷新打开的窗口"}},
		{"features", "features/Vim 程序", []string{"Vim 程序名", "添加 Vim 程序", "移除 Vim 程序 code.exe"}},
		{"dicts", "dicts/词库词条", []string{"保存名称与说明", "搜索词库词条", "词库上一页", "词库下一页", "删除选中词库词条"}},
		{"about", "about/检查更新", []string{"检查软件更新", "立即更新"}},
		{"candidates", "candidates/字体", []string{"字体", "恢复默认"}},
		{"candidates", "candidates/自定义配色", []string{"窗口背景", "选中背景", "选中文字", "恢复默认配色"}},
		{"typing", "typing/翻页键", []string{"- =", "[ ]", ", .", "Tab"}},
		{"sync", "sync/同步计划", []string{"保存同步计划", "立即同步", "打开恢复前备份", "今日自动尝试"}},
	}
	for _, scale := range []float32{1, 1.5} {
		for _, item := range cases {
			test.Run(fmt.Sprintf("%s/%s/scale-%.1f", item.page, item.target, scale), func(test *testing.T) {
				state, backend := newTestSettings()
				state.syncDir = test.TempDir()
				state.loadSync()
				state.sync.Enabled, state.sync.Automatic = true, true
				state.config.AppPresets = true
				state.config.VimModeApps = []string{"code.exe", "gvim.exe"}
				state.config.FontFamily = "SimSun"
				if item.target == "candidates/自定义配色" {
					state.config.CandidateStyle = "custom"
				}
				backend.dicts = []Dictionary{
					{Name: "专业词库", Entries: 120, Enabled: true, Source: "Rime · 医学.dict.yaml", Description: "日常医学用词"},
					{Name: "停用词库", Entries: 45, Enabled: false},
				}
				state.dicts = backend.dicts
				state.dictionaryName, state.dictionaryRename, state.dictionaryDescription = "专业词库", "专业词库", "日常医学用词"
				state.dictionaryPage = &DictionaryPage{
					Dictionary: backend.dicts[0],
					Total:      120,
					Entries: []DictionaryEntry{
						{Word: "研究", Pinyin: "yan jiu", Weight: 1000},
						{Word: "临床试验", Pinyin: "lin chuang shi yan", Weight: 800},
					},
				}
				state.dictionarySelected = map[string]bool{}
				state.locateSetting(setting(item.target))
				view := ui.NewTester(state.view, 780, 1400)
				view.SetPreferences(ui.Preferences{TextScale: scale})
				state.locateSetting(setting(item.target))
				view.Frame()
				for _, control := range item.controls {
					bounds, found := view.Find(control)
					if !found || bounds.W <= 1 || bounds.X < 236 || bounds.X+bounds.W > 756 {
						test.Errorf("%s 在最小宽度下必须完整保留：%+v，存在=%v", control, bounds, found)
					}
				}
				saveUIFrame(test, fmt.Sprintf("%s-narrow-scale-%.1f", strings.ReplaceAll(item.target, "/", "-"), scale), view)
			})
		}
	}
}

func TestSettingsSyncFormsUseAvailableWidthWithoutRightSideBlankGap(test *testing.T) {
	for _, provider := range []string{"webdav", "s3"} {
		test.Run(provider, func(test *testing.T) {
			state, _ := newTestSettings()
			state.syncDir = test.TempDir()
			state.loadSync()
			state.sync.Enabled, state.sync.Provider = true, provider
			state.locateSetting(setting("sync/同步服务"))
			view := ui.NewTester(state.view, 1280, 1000)
			view.Frame()
			label := "地址"
			if provider == "s3" {
				label = "服务地址"
			}
			for _, node := range view.AccessAll() {
				if node.Name == label && node.Focusable {
					if node.Bounds.W < 680 {
						test.Fatalf("%s 宽屏输入框不能留出右侧大片空白：%+v", provider, node.Bounds)
					}
					return
				}
			}
			test.Fatalf("未找到 %s 输入框", label)
		})
	}
}

func TestSettingsSyncInputsKeepTheCaretInsideNarrowPages(test *testing.T) {
	state, _ := newTestSettings()
	state.syncDir = test.TempDir()
	state.loadSync()
	state.sync.Enabled, state.sync.Provider = true, "s3"
	state.locateSetting(setting("sync/同步服务"))
	view := ui.NewTester(state.view, 780, 1400)
	if err := view.Click("Secret Key"); err != nil {
		test.Fatal(err)
	}
	view.Type(strings.Repeat("x", 80))
	caret, found := view.TextCaret()
	if !found || caret.X < 236 || caret.X+caret.W > 756 {
		test.Fatalf("长密钥的插入光标必须留在页面内：%+v，存在=%v", caret, found)
	}
	saveUIFrame(test, "sync-s3-long-input", view)
}

func TestSyncRecoveryActionsStayInsideNarrowPage(test *testing.T) {
	for _, scale := range []float32{1, 1.5} {
		test.Run(fmt.Sprintf("scale-%.1f", scale), func(test *testing.T) {
			state, _ := newTestSettings()
			state.syncDir = test.TempDir()
			state.loadSync()
			state.sync.Enabled, state.sync.Automatic = true, true
			state.autoSyncStatus = autoSyncState{Failed: true, Message: "同步记录与备份正文不一致"}
			state.confirmSyncResetCloud = true
			state.locateSetting(setting("sync/同步计划"))
			view := ui.NewTester(state.view, 780, 1400)
			view.SetPreferences(ui.Preferences{TextScale: scale})
			state.locateSetting(setting("sync/同步计划"))
			view.Frame()
			for _, label := range []string{"重建本机同步记录并重试", "查看云端备份", "以本机数据重置云端", "确认以本机重置云端"} {
				bounds, found := view.Find(label)
				if !found || bounds.W <= 1 || bounds.X < 236 || bounds.X+bounds.W > 756 {
					test.Errorf("%s 在窄屏同步故障恢复面板下越界：%+v，存在=%v", label, bounds, found)
				}
			}
		})
	}
}

func TestSettingsPageLayoutMatrix(test *testing.T) {
	for _, page := range pages {
		test.Run(page.ID, func(test *testing.T) {
			state, _ := newTestSettings()
			state.resources = os.Getenv("FREETYPE_UI_RESOURCES")
			state.syncDir = test.TempDir()
			state.loadSync()
			state.sync.Enabled, state.sync.Automatic, state.sync.Encrypt = true, true, true
			state.config.AppPresets, state.config.Fuzzy, state.config.Shuangpin, state.config.Autospace = true, true, true, true
			state.config.StatusBar, state.config.PredictNext, state.config.ClipboardHistory = true, true, true
			state.wordsLoaded = true
			state.words = []UserWord{{Word: "输入法", Pinyin: "shu ru fa", Freq: 3000}, {Word: "霜频阁", Pinyin: "shuang pin ge", Freq: 30000}}
			state.page = page.ID
			view := ui.NewTester(state.view, 1000, 900)
			for _, dark := range []bool{false, true} {
				for _, size := range []struct {
					name            string
					width, height   int
					text, zoom, dpi float32
				}{
					{"normal", 1000, 900, 1, 1, 1},
					{"minimum", 780, 540, 1, 1, 1},
					{"large-text", 780, 720, 1.5, 1, 1},
					{"zoom", 780, 720, 1, 1.25, 1},
					{"dpi", 1000, 900, 1, 1, 1.25},
				} {
					name := fmt.Sprintf("%s/%s/dark-%v", page.ID, size.name, dark)
					test.Run(fmt.Sprintf("%s/dark-%v", size.name, dark), func(test *testing.T) {
						state.pageScroll.X, state.pageScroll.Y = 0, 0
						view.SetSize(size.width, size.height)
						view.SetZoom(size.zoom)
						view.SetScale(size.dpi)
						view.SetDark(dark)
						view.SetPreferences(ui.Preferences{TextScale: size.text})
						view.Frame()
						search, found := view.Find("搜索全部设置")
						if !found || search.W < 1 || search.X < 0 || search.X+search.W > float32(size.width) || search.Y+search.H > float32(size.height)+1 {
							test.Errorf("缩放后必须保留完整的设置搜索入口：%s %+v", name, search)
						}
						if !view.HasText(page.Name) {
							test.Errorf("没有页面标题：%s", name)
						}
						saveUIFrame(test, strings.ReplaceAll(name, "/", "-"), view)
					})
				}
			}
		})
	}
}

func TestSyncPlanSettingsAlignAndIndentScheduleOption(test *testing.T) {
	state, _ := newTestSettings()
	state.syncDir = test.TempDir()
	state.loadSync()
	state.sync.Enabled, state.sync.Automatic = true, true
	state.page = "sync"

	for _, tc := range []struct {
		schedule string
		child    string
	}{
		{"interval", "同步间隔 / 分钟"},
		{"daily", "每天时间"},
	} {
		state.sync.Schedule = tc.schedule
		view := ui.NewTester(state.view, 1000, 1800)
		view.Frame()
		modeRow, ok := view.Find("同步方式")
		if !ok {
			test.Fatalf("同步方式设置项缺失")
		}
		childRow, ok := view.Find(tc.child)
		if !ok {
			test.Fatalf("%s 设置项缺失", tc.child)
		}
		if childRow.X <= modeRow.X+10 {
			test.Fatalf("%s 应作为三级子项相对「同步方式」缩进：mode.X=%.1f child.X=%.1f", tc.child, modeRow.X, childRow.X)
		}
		for _, sibling := range []string{"启动时同步", "每日最多次数", "失败重试次数", "每台电脑保留备份"} {
			rowBounds, found := view.Find(sibling)
			if !found {
				test.Fatalf("%s 设置项缺失", sibling)
			}
			if math.Abs(float64(rowBounds.X-modeRow.X)) > 1 {
				test.Fatalf("二级设置项 %s 与「同步方式」左边缘未对齐：got=%.1f want=%.1f", sibling, rowBounds.X, modeRow.X)
			}
		}
	}
}
