// Adapted from WowType settings/wowtype-config at bb2a04fd573f1e5f2444ee68f8f71b7ea429b3c5.
// Apache-2.0; see third_party/licenses/WowType-KEYBOARD-NOTICE.txt.
// FreeType changes: native mygo rendering, Windows Set-1 scan codes, theme and text scaling.
package main

import (
	"cmp"
	"fmt"
	"slices"

	"github.com/egoist/mygo/ui"
)

type physicalKey struct {
	scan  int
	label string
	units float32
}

// Port of upstream stats_rows: six ANSI rows, each 15 units wide; zero is a spacer.
// The extended scan bit distinguishes right modifiers and numpad/navigation keys.
var physicalKeyboard = [][]physicalKey{
	{{0x01, "Esc", 1}, {0, "", 1}, {0x3b, "F1", 1}, {0x3c, "F2", 1}, {0x3d, "F3", 1}, {0x3e, "F4", 1}, {0, "", .5},
		{0x3f, "F5", 1}, {0x40, "F6", 1}, {0x41, "F7", 1}, {0x42, "F8", 1}, {0, "", .5}, {0x43, "F9", 1}, {0x44, "F10", 1}, {0x57, "F11", 1}, {0x58, "F12", 1}},
	{{0x29, "`", 1}, {0x02, "1", 1}, {0x03, "2", 1}, {0x04, "3", 1}, {0x05, "4", 1}, {0x06, "5", 1}, {0x07, "6", 1},
		{0x08, "7", 1}, {0x09, "8", 1}, {0x0a, "9", 1}, {0x0b, "0", 1}, {0x0c, "-", 1}, {0x0d, "=", 1}, {0x0e, "Backspace", 2}},
	{{0x0f, "Tab", 1.5}, {0x10, "Q", 1}, {0x11, "W", 1}, {0x12, "E", 1}, {0x13, "R", 1}, {0x14, "T", 1}, {0x15, "Y", 1},
		{0x16, "U", 1}, {0x17, "I", 1}, {0x18, "O", 1}, {0x19, "P", 1}, {0x1a, "[", 1}, {0x1b, "]", 1}, {0x2b, "\\", 1.5}},
	{{0x3a, "Caps Lock", 1.75}, {0x1e, "A", 1}, {0x1f, "S", 1}, {0x20, "D", 1}, {0x21, "F", 1}, {0x22, "G", 1}, {0x23, "H", 1},
		{0x24, "J", 1}, {0x25, "K", 1}, {0x26, "L", 1}, {0x27, ";", 1}, {0x28, "'", 1}, {0x1c, "Enter", 2.25}},
	{{0x2a, "Shift", 2.25}, {0x2c, "Z", 1}, {0x2d, "X", 1}, {0x2e, "C", 1}, {0x2f, "V", 1}, {0x30, "B", 1}, {0x31, "N", 1},
		{0x32, "M", 1}, {0x33, ",", 1}, {0x34, ".", 1}, {0x35, "/", 1}, {0x36, "Shift", 2.75}},
	{{0x1d, "Ctrl", 1.25}, {0x15b, "Win", 1.25}, {0x38, "Alt", 1.25}, {0x39, "Space", 6.25}, {0x138, "Alt", 1.25},
		{0x15c, "Win", 1.25}, {0x15d, "Menu", 1.25}, {0x11d, "Ctrl", 1.25}},
}

func physicalKeyName(scan int) string {
	for _, row := range physicalKeyboard {
		for _, key := range row {
			if key.scan == scan && scan != 0 {
				return key.label
			}
		}
	}
	names := map[int]string{
		0x147: "Home", 0x148: "上箭头", 0x149: "Page Up", 0x14b: "左箭头", 0x14d: "右箭头", 0x14f: "End", 0x150: "下箭头",
		0x151: "Page Down", 0x152: "Insert", 0x153: "Delete", 0x137: "Print Screen", 0x45: "Num Lock", 0x46: "Scroll Lock",
		0x47: "小键盘 7", 0x48: "小键盘 8", 0x49: "小键盘 9", 0x4b: "小键盘 4", 0x4c: "小键盘 5", 0x4d: "小键盘 6",
		0x4f: "小键盘 1", 0x50: "小键盘 2", 0x51: "小键盘 3", 0x52: "小键盘 0", 0x53: "小键盘 .", 0x11c: "小键盘回车",
		0x37: "小键盘 *", 0x135: "小键盘 /", 0x4a: "小键盘 -", 0x4e: "小键盘 +", 0x56: "ISO \\",
	}
	if name := names[scan]; name != "" {
		return name
	}
	return fmt.Sprintf("键位 0x%03X", scan)
}

// Port of stats_heat_level, using division before multiplication to avoid uint64 overflow.
func keyboardHeatLevel(count, maximum uint64) int {
	if count == 0 || maximum == 0 {
		return 0
	}
	if count >= maximum {
		return 4
	}
	for level := uint64(1); level < 4; level++ {
		threshold := maximum/4*level + maximum%4*level/4
		if count <= threshold {
			return int(level)
		}
	}
	return 4
}

func keyboardHeatColors(c *ui.Context, level int) (ui.Color, ui.Color) {
	if c.Preferences().HighContrast {
		if level > 0 {
			return c.Theme().Accent, c.Theme().AccentText
		}
		return c.Theme().Background, c.Theme().Text
	}
	if c.Theme().Dark {
		background := [5]string{"#2b2b30", "#493044", "#704363", "#af71a0", "#e3a6d3"}
		foreground := [5]string{"#b4b4be", "#f6e5f1", "#ffffff", "#18181b", "#18181b"}
		return ui.Hex(background[level]), ui.Hex(foreground[level])
	}
	background := [5]string{"#f3f3f5", "#fae6f6", "#edbee4", "#c777b6", "#963b84"}
	foreground := [5]string{"#65656f", "#4b2544", "#4b2544", "#271324", "#ffffff"}
	return ui.Hex(background[level]), ui.Hex(foreground[level])
}

func statsKeyboard(c *ui.Context, keys map[int]uint64) {
	var total, peak uint64
	var ranked []int
	for scan, count := range keys {
		total += count
		peak = max(peak, count)
		if count > 0 {
			ranked = append(ranked, scan)
		}
	}
	slices.SortFunc(ranked, func(a, b int) int {
		if order := cmp.Compare(keys[b], keys[a]); order != 0 {
			return order
		}
		return cmp.Compare(a, b)
	})
	ui.Column(c).FillWidth().Padding(12, 0).Gap(14).Children(func() {
		ui.Text(c, fmt.Sprintf("共 %d 次按键 · %d 个键位", total, len(ranked))).FontWeight(500)
		if total == 0 {
			ui.Text(c, "暂无按键记录").TextColor(colors(c).muted)
		}
		drawn := map[int]bool{}
		ui.ScrollHorizontal(c).FillWidth().Children(func() {
			ui.Column(c).Shrink(0).Gap(4).Padding(8).Radius(8).Background(colors(c).subtle).Children(func() {
				for rowIndex, row := range physicalKeyboard {
					line := ui.Row(c).Gap(4).Shrink(0)
					if rowIndex == 0 {
						line.Margin(0, 0, 6, 0)
					}
					line.Children(func() {
						for _, key := range row {
							width, height := key.units*textSize(c, 40)-4, textSize(c, 38)
							if key.scan == 0 {
								ui.Box(c).Size(width, height)
								continue
							}
							drawn[key.scan] = true
							count := keys[key.scan]
							bg, fg := keyboardHeatColors(c, keyboardHeatLevel(count, peak))
							ui.Column(c).Label("键位："+key.label).Size(width, height).Shrink(0).Center().
								Background(bg).Border(1, colors(c).border).Radius(5).
								Tooltip(fmt.Sprintf("%s：%d 次", physicalKeyName(key.scan), count)).Children(func() {
								ui.Text(c, key.label).FontSize(textSize(c, 12)).FontWeight(500).TextColor(fg)
							})
						}
					})
				}
			})
		})
		ui.Row(c).Gap(5).AlignItems(ui.Center).Children(func() {
			ui.Text(c, "少").FontSize(textSize(c, 12)).TextColor(colors(c).muted)
			for level := 0; level < 5; level++ {
				bg, _ := keyboardHeatColors(c, level)
				ui.Box(c).Size(14, 14).Radius(3).Background(bg)
			}
			ui.Text(c, "多；按当前范围最高次数分级").FontSize(textSize(c, 12)).TextColor(colors(c).muted)
		})
		var others []int
		for _, scan := range ranked {
			if !drawn[scan] {
				others = append(others, scan)
			}
		}
		if len(others) > 0 {
			ui.Text(c, "其他键").FontWeight(500)
			ui.Row(c).Wrap().Gap(8).Children(func() {
				for _, scan := range others {
					ui.Row(c).Gap(8).Padding(6, 10).Radius(5).Background(colors(c).subtle).Children(func() {
						bg, _ := keyboardHeatColors(c, keyboardHeatLevel(keys[scan], peak))
						ui.Box(c).Size(10, 10).Radius(2).Background(bg)
						ui.Text(c, physicalKeyName(scan))
						ui.Text(c, fmt.Sprintf("%d 次", keys[scan])).TextColor(colors(c).muted)
					})
				}
			})
		}
		if len(ranked) > 0 {
			ui.Text(c, "使用最多的键").FontWeight(500)
			ui.Row(c).Wrap().Gap(16).Children(func() {
				for _, scan := range ranked[:min(5, len(ranked))] {
					ui.Text(c, fmt.Sprintf("%s  %d 次", physicalKeyName(scan), keys[scan])).FontSize(textSize(c, 13))
				}
			})
		}
		ui.Text(c, "仅统计 FreeType 处理的按键；未接管的按键不会计数；窄窗口可横向滚动").FontSize(textSize(c, 12)).TextColor(colors(c).muted).Wrap()
	})
}
