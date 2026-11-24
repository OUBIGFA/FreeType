package main

import (
	"image/color"
	"testing"

	"github.com/egoist/mygo/ui"
)

// 每个内置 Solar 图标都要被 mygo 完整支持，并且画出可见的线条；
// 否则换图后可能出现缺笔画或空白图标而不报错。
func TestSolarIconsRender(t *testing.T) {
	all := map[string]*ui.SVG{
		"general": icons.general, "candidates": icons.candidates, "typing": icons.typing,
		"features": icons.features, "shortcuts": icons.shortcuts, "dicts": icons.dicts,
		"phrases": icons.phrases, "common": icons.common, "learned": icons.learned,
		"stats": icons.stats, "sync": icons.sync, "about": icons.about, "statusCheck": statusCheck,
	}
	// ui.Icon 只用图形覆盖范围着色，这里用 Image 统计覆盖到的像素即可，颜色不参与判断。
	textColor := color.NRGBA{R: 0x20, G: 0x80, B: 0xd0, A: 0xff}
	for name, icon := range all {
		if unsupported := icon.Unsupported(); len(unsupported) > 0 {
			t.Errorf("%s 含 mygo 不支持的 SVG 特性：%v", name, unsupported)
		}
		if w, h := icon.Size(); w != 24 || h != 24 {
			t.Errorf("%s 尺寸为 %v×%v，应为 Solar 的 24×24", name, w, h)
		}
		img := icon.Image(48, 48, textColor)
		inked := 0
		for i := 0; i < len(img.Pix); i += 4 {
			if img.Pix[i+3] >= 0x80 {
				inked++
			}
		}
		if inked < 40 {
			t.Errorf("%s 几乎没有画出线条：%d 个像素", name, inked)
		}
	}
}
