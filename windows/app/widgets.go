package main

import (
	"github.com/egoist/mygo/ui"
)

// 设置窗口的样子照 Windows 11「设置」：浅灰底、白色圆角卡片，卡片里一行一项，
// 左边是名字和一句说明，右边是开关或下拉框；少用的选项在打开开关后才出现在下面。
// 开关、按钮这些强调色用品牌 logo 的粉色；左边的导航不用强调色，选中项是浅灰底。

// brandPink 是 logo 的粉色。
var brandPink = ui.Hex("#ffb3fa")

// branded 是加了品牌粉色强调色的主题（跟着系统的浅色 / 深色）。
func branded(c *ui.Context) *ui.Theme {
	t := *c.Theme().WithAccent(brandPink)
	return &t
}

// palette 是界面用的几种颜色，跟随系统浅色 / 深色。
type palette struct {
	page, card, cardHover, border, muted, subtle, chosen ui.Color
}

func colors(c *ui.Context) palette {
	if c.Theme().Dark {
		return palette{
			page: ui.Hex("#202020"), card: ui.Hex("#2b2b2b"), cardHover: ui.Hex("#323232"),
			border: ui.Hex("#383838"), muted: ui.Hex("#a0a0a0"), subtle: ui.Hex("#262626"), chosen: ui.Hex("#2d2d2d"),
		}
	}
	return palette{
		page: ui.Hex("#f3f3f3"), card: ui.Hex("#fbfbfb"), cardHover: ui.Hex("#f6f6f6"),
		border: ui.Hex("#e5e5e5"), muted: ui.Hex("#616161"), subtle: ui.Hex("#f9f9f9"), chosen: ui.Hex("#e8e8e8"),
	}
}

// stroked 把一组描边路径包成 24×24 的图标（颜色跟文字走）。
func stroked(paths string) *ui.SVG {
	return ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">` + paths + `</svg>`))
}

// 侧边栏图标（自己画的简单线条）。
var icons = struct {
	general, candidates, typing, features, shortcuts, dicts, phrases, learned, about *ui.SVG
}{
	general:    stroked(`<path d="M4 7h10M18 7h2M4 17h4M12 17h8"/><circle cx="16" cy="7" r="2"/><circle cx="10" cy="17" r="2"/>`),
	candidates: stroked(`<rect x="3" y="6" width="18" height="12" rx="2.5"/><path d="M7 12h3M13 12h4"/>`),
	typing:     stroked(`<rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>`),
	features:   stroked(`<path d="M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8z"/><path d="M18 15l.8 2.2L21 18l-2.2.8L18 21l-.8-2.2L15 18l2.2-.8z"/>`),
	shortcuts:  stroked(`<rect x="3" y="3" width="8" height="8" rx="2"/><rect x="13" y="13" width="8" height="8" rx="2"/><path d="M13 7h4a2 2 0 0 1 2 2v2M11 17H7a2 2 0 0 1-2-2v-2"/>`),
	dicts:      stroked(`<path d="M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2z"/><path d="M4 21V5M8 7h7M8 11h5"/>`),
	phrases:    stroked(`<path d="M4 5h16v11H9l-5 4z"/><path d="M8 9h8M8 12h5"/>`),
	learned:    stroked(`<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l3 2"/>`),
	about:      stroked(`<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5h.01"/>`),
}

// navItem 是左边导航的一项：图标和名字；选中时浅灰底。点了返回 true。
func navItem(c *ui.Context, icon *ui.SVG, name string, chosen bool) bool {
	p := colors(c)
	item := ui.Row(c).Height(36).Padding(0, 12).Gap(12).Radius(6).AlignItems(ui.Center).Cursor(ui.CursorPointer).Label(name)
	if chosen {
		item.Background(p.chosen)
	} else if item.Hovered() {
		item.Background(p.cardHover)
	}
	item.Children(func() {
		ui.Icon(c, icon).FontSize(16).TextColor(p.muted)
		ui.Text(c, name).SingleLine()
	})
	return item.Clicked()
}

// pageHeader 是页面顶上的标题和一句说明。
func pageHeader(c *ui.Context, title, subtitle string) {
	ui.Column(c).Gap(4).Margin(0, 0, 6, 0).Children(func() {
		ui.Text(c, title).FontSize(26).FontWeight(600)
		if subtitle != "" {
			ui.Text(c, subtitle).TextColor(colors(c).muted).Wrap()
		}
	})
}

// section 是一组设置：小标题加一张卡片，卡片里的行之间有分隔线。
func section(c *ui.Context, title string, fn func()) {
	p := colors(c)
	ui.Column(c).Gap(8).Margin(14, 0, 0, 0).Children(func() {
		if title != "" {
			ui.Text(c, title).FontSize(14).FontWeight(600)
		}
		ui.Column(c).Radius(8).Background(p.card).Border(1, p.border).Dividers(1, p.border).Clip().Children(fn)
	})
}

// texts 是一行左边的名字和说明。
func texts(c *ui.Context, title, desc string) ui.Element {
	return ui.Column(c).Grow(1).Shrink(1).Gap(2).Children(func() {
		ui.Text(c, title)
		if desc != "" {
			ui.Text(c, desc).FontSize(12.5).TextColor(colors(c).muted).Wrap()
		}
	})
}

// row 是卡片里的一行：左边名字和说明，右边 control 画的控件。
func row(c *ui.Context, title, desc string, control func()) {
	ui.Row(c).Padding(13, 18).Gap(24).AlignItems(ui.Center).MinHeight(60).Children(func() {
		texts(c, title, desc)
		if control != nil {
			ui.Row(c).Gap(8).AlignItems(ui.Center).Shrink(0).Children(control)
		}
	})
}

// subRow 是开关打开后出现在下面、缩进一级的一行。
func subRow(c *ui.Context, title, desc string, control func()) {
	ui.Row(c).Padding(11, 18, 11, 46).Gap(24).AlignItems(ui.Center).Background(colors(c).subtle).Children(func() {
		texts(c, title, desc)
		if control != nil {
			ui.Row(c).Gap(8).AlignItems(ui.Center).Shrink(0).Children(control)
		}
	})
}

// switchRow 是带开关的一行；点名字也能切换。返回这一帧有没有改。
func switchRow(c *ui.Context, title, desc string, value *bool) bool {
	return switchLine(c, title, desc, value, false)
}

func switchLine(c *ui.Context, title, desc string, value *bool, sub bool) bool {
	changed := false
	padding := []float32{13, 18}
	if sub {
		padding = []float32{11, 18, 11, 46}
	}
	line := ui.Row(c).Padding(padding...).Gap(24).AlignItems(ui.Center).MinHeight(52)
	if sub {
		line.Background(colors(c).subtle)
	}
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
			ui.Text(c, state).TextColor(colors(c).muted).FontSize(13)
			if ui.Switch(c, value).Label(title).Changed() {
				changed = true
			}
		})
	})
	return changed
}

// choose 是下拉框：value 存的是 ID，下拉框里显示名字。返回这一帧有没有改。
func choose(c *ui.Context, label string, list []choice, value *string, width float32) bool {
	name := nameOf(list, *value)
	if ui.Select(c, &name, names(list)).Label(label).Width(width).Changed() {
		*value = idOf(list, name)
		return true
	}
	return false
}

// hint 是卡片下面一行小字说明。
func hint(c *ui.Context, text string) {
	ui.Text(c, text).FontSize(12.5).TextColor(colors(c).muted).Wrap().Margin(6, 2, 0, 2)
}

// keyCap 画一个按键的样子（快捷键页用）。
func keyCap(c *ui.Context, text string) {
	p := colors(c)
	ui.Text(c, text).FontSize(12.5).Padding(2, 8).Radius(5).Border(1, p.border).Background(p.page)
}
