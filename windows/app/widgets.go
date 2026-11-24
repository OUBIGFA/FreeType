package main

import (
	"embed"

	"github.com/egoist/mygo/ui"
)

// 设置窗口采用分层卡片设计：
// 1. 同组一级设置之间用浅色细线分隔，左右与正文对齐；
// 2. 父项展开后的次级项统一收纳在柔和底色分组内，三级嵌套通过进一步缩进表达从属关系；
// 3. 只有真正放不下的操作组换行，不缩小文字或截掉操作。
const (
	controlRadius float32 = 6
	panelRadius   float32 = 10
	rowInset      float32 = 20
)

// 显式字号也跟随 Windows 的文字大小；Theme.Rem 已包含系统文字缩放。
func textSize(c *ui.Context, size float32) float32 { return c.Theme().Rem(size / 14) }

// brandPink 是 logo 的粉色。
var brandPink = ui.Hex("#ffb3fa")

// settingsTheme 让输入、选择、按钮和焦点提示共用中性深浅灰，避免局部主题遗漏控件。
// 输入框与下拉框使用更干净的底色，配合卡片表面形成清晰的立体对比。
func settingsTheme(c *ui.Context) *ui.Theme {
	theme := *c.Theme()
	theme.Radius = controlRadius
	if c.Preferences().HighContrast {
		return &theme
	}
	gray := ui.Hex("#27272a")
	if theme.Dark {
		gray = ui.Hex("#d4d4d8")
	}
	result := theme.WithAccent(gray)
	result.Focus = gray
	if theme.Dark {
		result.Surface = ui.Hex("#1f1f22")
		result.SurfaceHover = ui.Hex("#2c2c30")
		result.SurfacePressed = ui.Hex("#36363b")
		result.Border = ui.Hex("#3d3d43")
	} else {
		result.Surface = ui.Hex("#f4f4f6")
		result.SurfaceHover = ui.Hex("#ebebef")
		result.SurfacePressed = ui.Hex("#e0e0e6")
		result.Border = ui.Hex("#dcdce2")
	}
	return result
}

// checkbox 保留原生勾选、键盘和无障碍行为，只统一选中与悬停的颜色。
// 原生控件仅未勾选时有悬停边框，该边框沿用基础主题的灰色；选中后始终维持粉色。
func checkbox(c *ui.Context, checked *bool, label string) ui.Element {
	theme := *c.Theme()
	if *checked && !c.Preferences().HighContrast {
		theme.Accent, theme.AccentText = brandPink, ui.Hex("#18181b")
	}
	var control ui.Element
	ui.Themed(c, &theme).Background(ui.Transparent).Children(func() {
		control = ui.Checkbox(c, checked, label)
	})
	return control
}

// palette 是界面用的几种颜色，跟随系统浅色 / 深色。
type palette struct {
	page, card, cardHover, border, subBorder, muted, subtle, chosen ui.Color
}

func colors(c *ui.Context) palette {
	if c.Preferences().HighContrast {
		theme := c.Theme()
		return palette{page: theme.Background, card: theme.Background, cardHover: theme.SurfaceHover,
			border: theme.Text, subBorder: theme.Text, muted: theme.TextMuted, subtle: theme.Background, chosen: theme.SurfacePressed}
	}
	if c.Theme().Dark {
		return palette{
			page: ui.Hex("#161618"), card: ui.Hex("#222225"), cardHover: ui.Hex("#2a2a2e"),
			border: ui.Hex("#323237"), subBorder: ui.Hex("#2b2b30"),
			muted: ui.Hex("#a1a1aa"), subtle: ui.Hex("#1b1b1e"), chosen: ui.Hex("#303036"),
		}
	}
	return palette{
		page: ui.Hex("#f4f4f6"), card: ui.Hex("#ffffff"), cardHover: ui.Hex("#f8f8fa"),
		border: ui.Hex("#e6e6eb"), subBorder: ui.Hex("#e8e8ee"),
		muted: ui.Hex("#65656f"), subtle: ui.Hex("#f5f6f8"), chosen: ui.Hex("#e7e7ec"),
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

// navItem 是左边导航的一项：图标和名字；选中时浅灰底。点了返回 true。
func navItem(c *ui.Context, icon *ui.SVG, name string, chosen bool) bool {
	p := colors(c)
	item := ui.ButtonBase(c).MinHeight(max(36, textSize(c, 36))).Padding(8, 12).Gap(12).
		Radius(controlRadius).Justify(ui.Start).AlignItems(ui.Center).Cursor(ui.CursorPointer).Label(name)
	iconColor := p.muted
	if chosen {
		item.Background(p.chosen)
		iconColor = c.Theme().Text
	} else if item.Hovered() {
		item.Background(p.cardHover)
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

// pageHeader 是页面顶上的标题和一句说明。
func pageHeader(c *ui.Context, title, subtitle string) {
	ui.Column(c).Gap(6).Margin(0, 0, 12, 0).Children(func() {
		ui.Text(c, title).FontSize(textSize(c, 24)).FontWeight(600).Wrap()
		if subtitle != "" {
			ui.Text(c, subtitle).TextColor(colors(c).muted).LineHeight(1.45).Wrap()
		}
	})
}

// section 是一组设置：只在相邻项目之间画浅色细线，首尾不增加分隔线。
func section(c *ui.Context, title string, fn func()) {
	p := colors(c)
	before := float32(18)
	if title == "" {
		before = 6
	}
	ui.Column(c).Gap(8).Margin(before, 0, 0, 0).Children(func() {
		if title != "" {
			ui.Text(c, title).FontSize(textSize(c, 13)).FontWeight(600).TextColor(p.muted).Padding(0, 4).Wrap()
		}
		card := ui.Column(c).Padding(6, rowInset).
			Radius(panelRadius).Background(p.card).Border(1, p.border).Clip()
		if !c.Preferences().HighContrast {
			alpha := float32(0.03)
			if c.Theme().Dark {
				alpha = 0.18
			}
			card.Shadow(0, 1, 3, 0, ui.RGBA(0, 0, 0, alpha))
		}
		card.Children(func() { ui.Column(c).FillWidth().Gap(1).Dividers(1, p.border).Children(fn) })
	})
}

// texts 是一行左边的名字和说明。
func texts(c *ui.Context, title, desc string) ui.Element {
	return textsWithSize(c, title, desc, 14, 13)
}

func subTexts(c *ui.Context, title, desc string) ui.Element {
	return textsWithSize(c, title, desc, 13.5, 12.5)
}

func textsWithSize(c *ui.Context, title, desc string, titleSize, descSize float32) ui.Element {
	return ui.Column(c).Grow(1).Basis(200).MinWidth(160).Shrink(1).Gap(2).Children(func() {
		ui.Text(c, title).FontSize(textSize(c, titleSize)).FontWeight(500).LineHeight(1.4).Wrap()
		if desc != "" {
			ui.Text(c, desc).FontSize(textSize(c, descSize)).TextColor(colors(c).muted).LineHeight(1.45).Wrap()
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

// subGroup 用统一柔和底色与缩进承接父级开关展开后的所有二级项，
// 子项之间仅使用浅色细线分隔，不加侧边强调条，也不把每个子项画成孤立小卡片。
func subGroup(c *ui.Context, fn func()) {
	p := colors(c)
	border := p.subBorder
	if c.Preferences().HighContrast {
		border = p.border
	}
	ui.Column(c).FillWidth().Padding(0, 0, 12, 0).Children(func() {
		ui.Column(c).FillWidth().Padding(4, 14, 4, 16).Radius(controlRadius).Background(p.subtle).
			Border(1, border).Clip().Children(func() {
			ui.Column(c).FillWidth().Gap(1).Dividers(1, p.subBorder).Children(fn)
		})
	})
}

// nestedSubGroup 在二级分组内部呈现第三级从属关系：只用更深的左侧缩进与顶部细线，不加侧边条，也不叠加嵌套卡片。
func nestedSubGroup(c *ui.Context, fn func()) {
	p := colors(c)
	ui.Column(c).FillWidth().Padding(0, 0, 6, 20).Children(func() {
		ui.Column(c).FillWidth().Padding(2, 0, 0, 0).BorderWidth(1, 0, 0, 0).BorderColor(p.subBorder).Children(func() {
			ui.Column(c).FillWidth().Gap(1).Dividers(1, p.subBorder).Children(fn)
		})
	})
}

// subContent 用于次级分组内的表单、勾选网格等非单行内容，保持统一上下留白。
func subContent(c *ui.Context, fn func()) {
	ui.Column(c).FillWidth().Padding(10, 0).Gap(12).Children(fn)
}

// row 是卡片里的一行：左边名字和说明，右边 control 画的控件。
func row(c *ui.Context, title, desc string, control func()) {
	ui.Row(c).Padding(11, 0).Gap(16).Wrap().AlignItems(ui.Center).MinHeight(58).Children(func() {
		texts(c, title, desc)
		if control != nil {
			ui.Row(c).Gap(8).Wrap().AlignItems(ui.Center).MinWidth(0).MaxWidthPercent(100).Shrink(1).Children(control)
		}
	})
}

// subRow 是次级分组里的一行：行高和字阶略轻于一级设置项，不重复套独立卡片。
func subRow(c *ui.Context, title, desc string, control func()) {
	ui.Row(c).FillWidth().Padding(8, 0).Gap(16).Wrap().MinHeight(46).AlignItems(ui.Center).Children(func() {
		subTexts(c, title, desc)
		if control != nil {
			ui.Row(c).Gap(8).Wrap().AlignItems(ui.Center).MinWidth(0).MaxWidthPercent(100).Shrink(1).Children(control)
		}
	})
}

// switchRow 是带开关的一行；点名字也能切换。返回这一帧有没有改。
func switchRow(c *ui.Context, title, desc string, value *bool) bool {
	return switchLine(c, title, desc, value, false)
}

func switchLine(c *ui.Context, title, desc string, value *bool, sub bool) bool {
	changed := false
	build := func(line ui.Element, labelFn func(*ui.Context, string, string) ui.Element) {
		line.Children(func() {
			left := labelFn(c, title, desc).Cursor(ui.CursorPointer)
			if left.Clicked() {
				*value = !*value
				changed = true
			}
			ui.Row(c).Gap(10).AlignItems(ui.Center).Shrink(0).Children(func() {
				state := "关"
				if *value {
					state = "开"
				}
				ui.Text(c, state).TextColor(colors(c).muted).FontSize(textSize(c, 13)).Width(textSize(c, 14)).TextAlign(ui.Center)
				if ui.Switch(c, value).Label(title).Changed() {
					changed = true
				}
			})
		})
	}
	if sub {
		build(ui.Row(c).FillWidth().Padding(8, 0).Gap(16).Wrap().AlignItems(ui.Center).MinHeight(46), subTexts)
		return changed
	}
	build(ui.Row(c).FillWidth().Padding(11, 0).Gap(16).Wrap().AlignItems(ui.Center).MinHeight(58), texts)
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

// hint 是卡片下面一行小字说明。
func hint(c *ui.Context, text string) {
	ui.Text(c, text).FontSize(textSize(c, 13)).TextColor(colors(c).muted).LineHeight(1.45).Padding(0, 4).Wrap().Margin(6, 0, 0, 0)
}

// keyCap 画一个按键的样子（快捷键页和短语编码用）。
func keyCap(c *ui.Context, text string) ui.Element {
	p := colors(c)
	return ui.Text(c, text).FontSize(textSize(c, 12)).FontWeight(500).Padding(4, 8).Radius(controlRadius).
		Border(1, p.border).Background(p.subtle).SingleLine().MaxWidthPercent(100)
}
