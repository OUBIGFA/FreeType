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

func TestCandidateSelectionTextIsReadableInEveryAppearance(test *testing.T) {
	for _, dark := range []bool{false, true} {
		theme := ui.LightTheme()
		if dark {
			theme = ui.DarkTheme()
		}
		for _, id := range []string{"pink", "blue", "green", "sogou"} {
			style := candidateStyle(id, dark)
			if id == "green" && (!style.block || style.fill != ui.Hex("#03dd6d")) {
				test.Errorf("dark=%v：微信绿预览必须使用 #03DD6D 色块，得到 %+v", dark, style)
			}
			background := theme.Background
			if style.block {
				background = style.fill
			}
			if contrast := colorContrast(style.text, background); contrast < 4.5 {
				test.Errorf("%s dark=%v：小字号候选文字对比度 %.2f:1，需至少 4.5:1", id, dark, contrast)
			}
		}
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
		{"about", "about/检查更新", []string{"检查软件更新", "立即更新"}},
		{"candidates", "candidates/字体", []string{"字体", "恢复默认"}},
		{"typing", "typing/翻页键", []string{"- =", "[ ]", ", .", "Tab"}},
		{"sync", "sync/同步计划", []string{"保存同步计划", "立即同步", "打开恢复前备份", "今日自动尝试"}},
	}
	for _, scale := range []float32{1, 1.5} {
		for _, item := range cases {
			test.Run(fmt.Sprintf("%s/scale-%.1f", item.page, scale), func(test *testing.T) {
				state, _ := newTestSettings()
				state.syncDir = test.TempDir()
				state.loadSync()
				state.sync.Enabled, state.sync.Automatic = true, true
				state.config.AppPresets = true
				state.config.FontFamily = "SimSun"
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
				saveUIFrame(test, fmt.Sprintf("%s-narrow-scale-%.1f", item.page, scale), view)
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
