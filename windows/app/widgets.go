package main

import (
	"github.com/egoist/mygo/ui"
)

// 设置窗口采用现代商业软件的分层卡片设计：
// 1. 卡片内部用留白与子块微色块组织层级，不用贯穿全宽的生硬线条；
// 2. 同级控件共用圆角、字阶和内边距；
// 3. 只有真正放不下的操作组换行，不缩小文字或截掉操作。
const (
	controlRadius float32 = 6
	panelRadius   float32 = 10
	rowInset      float32 = 20
	subInset      float32 = 36
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
	page, card, cardHover, border, muted, subtle, chosen ui.Color
}

func colors(c *ui.Context) palette {
	if c.Preferences().HighContrast {
		theme := c.Theme()
		return palette{page: theme.Background, card: theme.Background, cardHover: theme.SurfaceHover,
			border: theme.Text, muted: theme.TextMuted, subtle: theme.Background, chosen: theme.SurfacePressed}
	}
	if c.Theme().Dark {
		return palette{
			page: ui.Hex("#161618"), card: ui.Hex("#222225"), cardHover: ui.Hex("#2a2a2e"),
			border: ui.Hex("#323237"), muted: ui.Hex("#a1a1aa"), subtle: ui.Hex("#1b1b1e"), chosen: ui.Hex("#303036"),
		}
	}
	return palette{
		page: ui.Hex("#f4f4f6"), card: ui.Hex("#ffffff"), cardHover: ui.Hex("#f8f8fa"),
		border: ui.Hex("#e6e6eb"), muted: ui.Hex("#65656f"), subtle: ui.Hex("#f7f7f9"), chosen: ui.Hex("#e7e7ec"),
	}
}

// stroked 把一组描边路径包成 24×24 的图标（颜色跟文字走）。
func stroked(paths string) *ui.SVG {
	return ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">` + paths + `</svg>`))
}

// 侧边栏图标（自己画的简单线条）。
var icons = struct {
	general, candidates, typing, features, shortcuts, dicts, phrases, common, learned, stats, sync, about *ui.SVG
}{
	general:    stroked(`<path d="M4 7h10M18 7h2M4 17h4M12 17h8"/><circle cx="16" cy="7" r="2"/><circle cx="10" cy="17" r="2"/>`),
	candidates: stroked(`<rect x="3" y="6" width="18" height="12" rx="2.5"/><path d="M7 12h3M13 12h4"/>`),
	typing:     stroked(`<rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>`),
	features:   stroked(`<path d="M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8z"/><path d="M18 15l.8 2.2L21 18l-2.2.8L18 21l-.8-2.2L15 18l2.2-.8z"/>`),
	shortcuts:  stroked(`<rect x="3" y="3" width="8" height="8" rx="2"/><rect x="13" y="13" width="8" height="8" rx="2"/><path d="M13 7h4a2 2 0 0 1 2 2v2M11 17H7a2 2 0 0 1-2-2v-2"/>`),
	dicts:      stroked(`<path d="M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2z"/><path d="M4 21V5M8 7h7M8 11h5"/>`),
	phrases:    stroked(`<path d="M4 5h16v11H9l-5 4z"/><path d="M8 9h8M8 12h5"/>`),
	common:     stroked(`<path d="M4 4h16v14H8l-4 3z"/><path d="M8 8h8M8 11h8M8 14h5"/>`),
	learned:    stroked(`<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l3 2"/>`),
	stats:      stroked(`<path d="M3 3v18h18M8 16v-4m5 4V8m5 8v-6"/>`),
	sync:       stroked(`<path d="M7 18a4 4 0 0 1-.6-7.96A6 6 0 0 1 18 9.5a4.25 4.25 0 0 1-.5 8.5"/><path d="M12 12v8M9 15l3-3 3 3"/>`),
	about:      stroked(`<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5h.01"/>`),
}

var statusCheck = stroked(`<path d="M5 12l4 4L19 6"/>`)

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

// section 是一组设置：小标题加一张卡片，卡片内部用留白和悬停底色代替生硬的全宽线条。
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
		card := ui.Column(c).Padding(6, 0).Radius(panelRadius).Background(p.card).Border(1, p.border).Clip()
		if !c.Preferences().HighContrast {
			alpha := float32(0.03)
			if c.Theme().Dark {
				alpha = 0.18
			}
			card.Shadow(0, 1, 3, 0, ui.RGBA(0, 0, 0, alpha))
		}
		card.Children(fn)
	})
}

// texts 是一行左边的名字和说明。
func texts(c *ui.Context, title, desc string) ui.Element {
	return ui.Column(c).Grow(1).Basis(200).MinWidth(160).Shrink(1).Gap(3).Children(func() {
		ui.Text(c, title).FontWeight(500).LineHeight(1.4).Wrap()
		if desc != "" {
			ui.Text(c, desc).FontSize(textSize(c, 13)).TextColor(colors(c).muted).LineHeight(1.45).Wrap()
		}
	})
}

// row 是卡片里的一行：左边名字和说明，右边 control 画的控件。
func row(c *ui.Context, title, desc string, control func()) {
	ui.Row(c).Padding(11, rowInset).Gap(16).Wrap().AlignItems(ui.Center).MinHeight(58).Children(func() {
		texts(c, title, desc)
		if control != nil {
			ui.Row(c).Gap(8).Wrap().AlignItems(ui.Center).MinWidth(0).MaxWidthPercent(100).Shrink(1).Children(control)
		}
	})
}

// subRow 是开关打开后出现在下面、缩进一级的一行，以柔和内嵌微色块呈现从属关系。
func subRow(c *ui.Context, title, desc string, control func()) {
	ui.Column(c).FillWidth().Padding(2, 12).Children(func() {
		ui.Row(c).FillWidth().Padding(10, rowInset-12, 10, subInset-12).Radius(controlRadius).Gap(16).Wrap().MinHeight(54).
			AlignItems(ui.Center).Background(colors(c).subtle).Children(func() {
			texts(c, title, desc)
			if control != nil {
				ui.Row(c).Gap(8).Wrap().AlignItems(ui.Center).MinWidth(0).MaxWidthPercent(100).Shrink(1).Children(control)
			}
		})
	})
}

// switchRow 是带开关的一行；点名字也能切换。返回这一帧有没有改。
func switchRow(c *ui.Context, title, desc string, value *bool) bool {
	return switchLine(c, title, desc, value, false)
}

func switchLine(c *ui.Context, title, desc string, value *bool, sub bool) bool {
	changed := false
	build := func(line ui.Element) {
		line.Children(func() {
			left := texts(c, title, desc).Cursor(ui.CursorPointer)
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
		ui.Column(c).FillWidth().Padding(2, 12).Children(func() {
			build(ui.Row(c).FillWidth().Padding(10, rowInset-12, 10, subInset-12).Radius(controlRadius).
				Gap(16).Wrap().AlignItems(ui.Center).MinHeight(54).Background(colors(c).subtle))
		})
		return changed
	}
	build(ui.Row(c).FillWidth().Padding(11, rowInset).Gap(16).Wrap().AlignItems(ui.Center).MinHeight(58))
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
