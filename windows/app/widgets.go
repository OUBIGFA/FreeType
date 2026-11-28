package main

import (
	"embed"
	"time"

	"github.com/egoist/mygo/ui"
)

// 设置窗口的设计令牌。取值和用法以 docs/UI-DESIGN.md 为准，改这里须同步改文档：
//  1. 间距只取 4 的倍数；分组面板左右各留 rowInset（项目约定 20 像素），面板内的行、
//     分组标题和面板下的说明都与这条边对齐；
//  2. 同组设置之间只画一条浅色细线；子项靠缩进表达从属，不再套底色框；
//  3. 分组面板无阴影；只有弹窗、菜单等浮层使用 mygo 默认的一层淡阴影；
//  4. 中性色占绝大部分面积，品牌粉 brandPink 是唯一强调色。
const (
	space1  float32 = 4
	space2  float32 = 8
	space3  float32 = 12
	space4  float32 = 16
	space5  float32 = 20
	space6  float32 = 24
	space8  float32 = 32
	space10 float32 = 40
	space12 float32 = 48

	// 小控件（按钮、输入框、导航项、键帽）与分组面板的圆角。
	controlRadius float32 = 8
	panelRadius   float32 = 16
	rowInset      float32 = space5
	// subIndent 是每深一级子项的缩进。
	subIndent float32 = space6

	// 字阶：页面标题、弹窗与状态标题、分组标题、正文、说明。说明是最小字号，不再用更小的字。
	fontTitle   float32 = 28
	fontHeading float32 = 17
	fontSection float32 = 15
	fontBody    float32 = 14
	fontCaption float32 = 13

	// 次要操作淡入淡出的时长，取 150–250ms 区间。
	revealDuration = 160 * time.Millisecond
)

// 显式字号也跟随 Windows 的文字大小；Theme.Rem 已包含系统文字缩放。
func textSize(c *ui.Context, size float32) float32 { return c.Theme().Rem(size / 14) }

// brandPink 是 logo 的粉色，也是界面唯一的强调色：勾选、开关、主按钮和关键数据。
// 它偏浅，上面的字固定用深色 inkOnPink。
var (
	brandPink = ui.Hex("#ffb3fa")
	inkOnPink = ui.Hex("#18181b")
)

// settingsTheme 统一控件底色、边框、文字和强调色，避免局部主题遗漏控件。
// 焦点环保持中性深灰，与粉色的“已选中”区分开。
func settingsTheme(c *ui.Context) *ui.Theme {
	theme := *c.Theme()
	theme.Radius = controlRadius
	if c.Preferences().HighContrast {
		return &theme
	}
	p := colors(c)
	result := theme.WithAccent(brandPink)
	result.AccentText = inkOnPink
	result.Text, result.TextMuted = p.text, p.muted
	if theme.Dark {
		result.Focus = ui.Hex("#d4d4d8")
		result.Surface = ui.Hex("#2c2c30")
		result.SurfaceHover = ui.Hex("#36363b")
		result.SurfacePressed = ui.Hex("#404046")
		result.Border = ui.Hex("#4a4a50")
	} else {
		result.Focus = ui.Hex("#3f3f46")
		result.Surface = ui.Hex("#f2f2f4")
		result.SurfaceHover = ui.Hex("#e8e8eb")
		result.SurfacePressed = ui.Hex("#dedee2")
		result.Border = ui.Hex("#cfcfd4")
	}
	return result
}

// checkbox 保留原生勾选、键盘和无障碍行为。未勾选时的悬停边框用中性灰，
// 勾选后才用强调色，避免鼠标划过一排复选框时到处闪粉色。
func checkbox(c *ui.Context, checked *bool, label string) ui.Element {
	theme := *c.Theme()
	if !*checked && !c.Preferences().HighContrast {
		theme.Accent = theme.Focus
	}
	var control ui.Element
	ui.Themed(c, &theme).Background(ui.Transparent).Children(func() {
		control = ui.Checkbox(c, checked, label)
	})
	return control
}

// palette 是界面用的中性色，跟随系统浅色 / 深色。
// pageHover 是页面底色上的悬停色（侧边栏），cardHover 是面板上的悬停色；
// border 是分隔线和小框的边，subtle 是键帽、标签这类小块的底色。
type palette struct {
	page, pageHover, card, cardHover, border, text, muted, subtle, chosen ui.Color
}

func colors(c *ui.Context) palette {
	if c.Preferences().HighContrast {
		theme := c.Theme()
		return palette{page: theme.Background, pageHover: theme.SurfaceHover, card: theme.Background, cardHover: theme.SurfaceHover,
			border: theme.Text, text: theme.Text, muted: theme.TextMuted, subtle: theme.Background, chosen: theme.SurfacePressed}
	}
	if c.Theme().Dark {
		return palette{
			page: ui.Hex("#161618"), pageHover: ui.Hex("#222226"), card: ui.Hex("#222225"), cardHover: ui.Hex("#2a2a2e"),
			border: ui.Hex("#36363b"), text: ui.Hex("#f2f2f2"), muted: ui.Hex("#a3a3a3"),
			subtle: ui.Hex("#2c2c30"), chosen: ui.Hex("#2e2e33"),
		}
	}
	return palette{
		page: ui.Hex("#f5f5f7"), pageHover: ui.Hex("#ececf0"), card: ui.Hex("#ffffff"), cardHover: ui.Hex("#f5f5f7"),
		border: ui.Hex("#dedee2"), text: ui.Hex("#181818"), muted: ui.Hex("#6a6a6a"),
		subtle: ui.Hex("#f2f2f4"), chosen: ui.Hex("#e6e6ea"),
	}
}

// 界面图标取自 Solar Icons 的 Linear 风格，SVG 原样保存在 assets/solar，
// 许可与署名文本随应用一起嵌入。ui.Icon 只取图形覆盖范围并按文字颜色着色，
// 所以原文件里的 stroke="black" 不必改写，深浅主题都跟随文字颜色。
//
//go:embed assets/solar/*.svg assets/solar-icons-*.txt
var solarAssets embed.FS

// solar 读取一个内置的 Solar Linear 图标；缺文件属于构建错误，启动即报出。
func solar(name string) *ui.SVG {
	data, err := solarAssets.ReadFile("assets/solar/" + name + ".svg")
	if err != nil {
		panic(err)
	}
	return ui.MustParseSVG(data)
}

// 侧边栏图标，按页面含义选取 Solar Linear 图标。
var icons = struct {
	general, candidates, typing, features, shortcuts, dicts, phrases, common, learned, stats, sync, about *ui.SVG
}{
	general:    solar("settings"),
	candidates: solar("palette"),
	typing:     solar("keyboard"),
	features:   solar("stars-minimalistic"),
	shortcuts:  solar("command"),
	dicts:      solar("book"),
	phrases:    solar("chat-round-line"),
	common:     solar("clipboard-text"),
	learned:    solar("history"),
	stats:      solar("chart-2"),
	sync:       solar("cloud-upload"),
	about:      solar("info-circle"),
}

var statusCheck = solar("check")

// navItem 是左边导航的一项：图标和名字；选中时浅灰底、字加粗。点了返回 true。
func navItem(c *ui.Context, icon *ui.SVG, name string, chosen bool) bool {
	p := colors(c)
	item := ui.ButtonBase(c).MinHeight(max(space10, textSize(c, space10))).Padding(space2, space3).Gap(space3).
		Radius(controlRadius).Justify(ui.Start).AlignItems(ui.Center).Cursor(ui.CursorPointer).Label(name)
	iconColor := p.muted
	if chosen {
		item.Background(p.chosen)
		iconColor = p.text
	} else if item.Hovered() {
		item.Background(p.pageHover)
	}
	item.Children(func() {
		ui.Icon(c, icon).FontSize(textSize(c, 16)).TextColor(iconColor)
		label := ui.Text(c, name).SingleLine()
		if chosen {
			label.FontWeight(600)
		}
	})
	return item.Clicked()
}

// pageHeader 是页面标题；note 只写使用前提、限制或隐私范围，没有就留空。
func pageHeader(c *ui.Context, title, note string) {
	ui.Column(c).Gap(space2).Children(func() {
		ui.Text(c, title).FontSize(textSize(c, fontTitle)).FontWeight(600).Wrap()
		if note != "" {
			ui.Text(c, note).TextColor(colors(c).muted).LineHeight(1.5).Wrap()
		}
	})
}

// panel 是一块分组面板：白底（深色为浅一级的灰）、大圆角、无边框无阴影；
// 高对比度主题下补一圈边框，保证面板轮廓可见。面板里不再嵌套面板。
func panel(c *ui.Context) ui.Element {
	p := colors(c)
	box := ui.Column(c).FillWidth().Radius(panelRadius).Background(p.card).Clip()
	if c.Preferences().HighContrast {
		box.Border(1, p.border)
	}
	return box
}

// sectionTitle 是分组标题，与面板内的文字左对齐。
func sectionTitle(c *ui.Context, title string) ui.Element {
	return ui.Text(c, title).FontSize(textSize(c, fontSection)).FontWeight(600).Margin(0, rowInset).Wrap()
}

// section 是一组设置：只在相邻项目之间画浅色细线，首尾不增加分隔线。
// 有标题的分组与上方内容拉开 32 像素（含页面列的 8 像素间隙），无标题的拉开 16 像素。
func section(c *ui.Context, title string, fn func()) {
	p := colors(c)
	before := space6
	if title == "" {
		before = space2
	}
	ui.Column(c).Gap(space3).Margin(before, 0, 0, 0).Children(func() {
		if title != "" {
			sectionTitle(c, title)
		}
		panel(c).Padding(space1, rowInset).Children(func() {
			ui.Column(c).FillWidth().Gap(1).Dividers(1, p.border).Children(fn)
		})
	})
}

// listHeader 是面板外、列表上方的一行：左边标题（常带条数），右边搜索框或按钮。
func listHeader(c *ui.Context, title string, controls func()) {
	ui.Row(c).Margin(space6, 0, 0, 0).Padding(0, 0, 0, rowInset).Gap(space3).Wrap().AlignItems(ui.Center).Children(func() {
		ui.Text(c, title).FontSize(textSize(c, fontSection)).FontWeight(600).Grow(1).Basis(180).Wrap()
		controls()
	})
}

// texts 是一行左边的名字和说明。
func texts(c *ui.Context, title, desc string) ui.Element {
	return textsWithWeight(c, title, desc, 500)
}

// subTexts 是子项的名字和说明：字号相同，标题改用常规字重，层级靠缩进表达。
func subTexts(c *ui.Context, title, desc string) ui.Element {
	return textsWithWeight(c, title, desc, 400)
}

func textsWithWeight(c *ui.Context, title, desc string, weight int) ui.Element {
	return ui.Column(c).Grow(1).Basis(200).MinWidth(160).Shrink(1).Gap(space1).Children(func() {
		ui.Text(c, title).FontSize(textSize(c, fontBody)).FontWeight(weight).LineHeight(1.4).Wrap()
		if desc != "" {
			ui.Text(c, desc).FontSize(textSize(c, fontCaption)).TextColor(colors(c).muted).LineHeight(1.5).Wrap()
		}
	})
}

// settingGroup 把父级设置项与它的次级项包在同一个区块单元内，
// 避免 section 的一级分隔线把父项和从属子项切开。
func settingGroup(c *ui.Context, parent func(), hasSub bool, sub func()) {
	ui.Column(c).FillWidth().Children(func() {
		parent()
		if hasSub && sub != nil {
			subGroup(c, sub)
		}
	})
}

// subLevel 是缩进一级的子项区：顶部和子项之间各一条细线，细线从缩进处开始，
// 不画底色框、侧边条或独立小卡片（docs/UI-DESIGN.md「层级」）。
func subLevel(c *ui.Context, fn func()) {
	p := colors(c)
	ui.Column(c).FillWidth().Padding(0, 0, 0, subIndent).Children(func() {
		ui.Column(c).FillWidth().BorderWidth(1, 0, 0, 0).BorderColor(p.border).Children(func() {
			ui.Column(c).FillWidth().Gap(1).Dividers(1, p.border).Children(fn)
		})
	})
}

// subGroup 承接父级开关展开后的所有二级项。
func subGroup(c *ui.Context, fn func()) { subLevel(c, fn) }

// nestedSubGroup 在二级子项内部呈现第三级从属关系，再缩进一级。
func nestedSubGroup(c *ui.Context, fn func()) { subLevel(c, fn) }

// subContent 用于次级分组内的表单、勾选网格等非单行内容，保持统一上下留白。
func subContent(c *ui.Context, fn func()) {
	ui.Column(c).FillWidth().Padding(space3, 0).Gap(space3).Children(fn)
}

// 一级设置行和子项行的上下内边距与最小高度。
const (
	rowPadding    = space4
	rowMinHeight  = 64
	subRowPadding = space3
	subRowMin     = 52
)

// line 是设置行的外框：左边文字，右边控件；放不下时控件整组换到下一行。
func line(c *ui.Context, sub bool) ui.Element {
	if sub {
		return ui.Row(c).FillWidth().Padding(subRowPadding, 0).Gap(space4).Wrap().AlignItems(ui.Center).MinHeight(subRowMin)
	}
	return ui.Row(c).FillWidth().Padding(rowPadding, 0).Gap(space4).Wrap().AlignItems(ui.Center).MinHeight(rowMinHeight)
}

func controls(c *ui.Context, control func()) {
	if control != nil {
		ui.Row(c).Gap(space2).Wrap().AlignItems(ui.Center).MinWidth(0).MaxWidthPercent(100).Shrink(1).Children(control)
	}
}

// row 是面板里的一行：左边名字和说明，右边 control 画的控件。
func row(c *ui.Context, title, desc string, control func()) {
	line(c, false).Children(func() {
		texts(c, title, desc)
		controls(c, control)
	})
}

// subRow 是子项区里的一行。
func subRow(c *ui.Context, title, desc string, control func()) {
	line(c, true).Children(func() {
		subTexts(c, title, desc)
		controls(c, control)
	})
}

// switchRow 是带开关的一行；点名字也能切换。返回这一帧有没有改。
func switchRow(c *ui.Context, title, desc string, value *bool) bool {
	return switchLine(c, title, desc, value, false)
}

// switchLine 不在开关旁另写「开 / 关」：开关本身的位置和强调色已经表示状态。
func switchLine(c *ui.Context, title, desc string, value *bool, sub bool) bool {
	changed := false
	labelFn := texts
	if sub {
		labelFn = subTexts
	}
	line(c, sub).Children(func() {
		if labelFn(c, title, desc).Cursor(ui.CursorPointer).Clicked() {
			*value = !*value
			changed = true
		}
		if ui.Switch(c, value).Label(title).Shrink(0).Changed() {
			changed = true
		}
	})
	return changed
}

// choose 是下拉框：value 存的是 ID，下拉框里显示名字。返回这一帧有没有改。
func choose(c *ui.Context, label string, list []choice, value *string, width float32) bool {
	name := nameOf(list, *value)
	if ui.Select(c, &name, names(list)).Label(label).Width(width).MaxWidthPercent(100).Shrink(1).Changed() {
		*value = idOf(list, name)
		return true
	}
	return false
}

// hint 是面板下面的一行说明，只写前提、限制和后果，与面板内文字左对齐。
func hint(c *ui.Context, text string) {
	ui.Text(c, text).FontSize(textSize(c, fontCaption)).TextColor(colors(c).muted).LineHeight(1.5).Margin(0, rowInset).Wrap()
}

// caption 是面板内的一段说明或空状态文字。
func caption(c *ui.Context, text string) ui.Element {
	return ui.Text(c, text).FontSize(textSize(c, fontCaption)).TextColor(colors(c).muted).LineHeight(1.5).Wrap()
}

// keyCap 画一个按键的样子（快捷键页和短语编码用）。
func keyCap(c *ui.Context, text string) ui.Element {
	p := colors(c)
	return ui.Text(c, text).FontSize(textSize(c, fontCaption)).FontWeight(500).Padding(space1, space2).Radius(controlRadius).
		Border(1, p.border).Background(p.subtle).SingleLine().MaxWidthPercent(100)
}

// onIntent 让列表行里的次要操作（如单条删除）平时隐藏，指针悬停或键盘焦点进入该行时淡入。
// 控件始终留在布局和 Tab 顺序里，键盘聚焦后行的 FocusWithin 为真，随即显示。
func onIntent(c *ui.Context, item ui.Element, fn func()) {
	target := float32(0)
	if item.Hovered() || item.FocusWithin() {
		target = 1
	}
	ui.Row(c).Shrink(0).Opacity(item.Animate("intent", target, revealDuration)).Children(fn)
}
