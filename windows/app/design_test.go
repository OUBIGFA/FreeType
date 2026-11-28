package main

import (
	"image"
	"math"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// 设计约定的回归测试，对应 docs/UI-DESIGN.md。只检查真实渲染出的位置和像素，不比对源码。

// visiblePixels 统计区域内与面板底色不同的像素：次要操作隐藏时应为 0。
func visiblePixels(view *ui.Tester, r ui.Rect, background ui.Color) int {
	img := view.Image()
	scale := float32(img.Bounds().Dx()) / 1000
	area := image.Rect(int(r.X*scale), int(r.Y*scale), int((r.X+r.W)*scale), int((r.Y+r.H)*scale)).Intersect(img.Bounds())
	count := 0
	for y := area.Min.Y; y < area.Max.Y; y++ {
		for x := area.Min.X; x < area.Max.X; x++ {
			c := img.RGBAAt(x, y)
			if c.R != background.R || c.G != background.G || c.B != background.B {
				count++
			}
		}
	}
	return count
}

func TestListRowSecondaryActionRevealsOnHoverAndStaysReachable(t *testing.T) {
	state, _ := newTestSettings()
	state.page = "learned"
	state.wordsLoaded = true
	state.words = []UserWord{{Word: "输入法", Pinyin: "shu ru fa", Freq: 3000}}
	view := ui.NewTester(state.view, 1000, 1100)
	view.Frame()
	button, found := view.Find("删除 输入法")
	if !found {
		t.Fatal("单条删除按钮必须留在界面里，供键盘和辅助技术使用")
	}
	reachable := false
	for _, control := range view.AccessAll() {
		reachable = reachable || (control.Name == "删除 输入法" && control.Focusable)
	}
	if !reachable {
		t.Fatal("隐藏时单条删除按钮也必须能用 Tab 聚焦")
	}
	card := ui.Hex("#ffffff")
	if n := visiblePixels(view, button, card); n != 0 {
		t.Fatalf("指针不在行上时单条删除按钮应隐藏，看到 %d 个像素", n)
	}
	view.Move(button.X-200, button.Y+button.H/2)
	view.Advance(300 * time.Millisecond)
	view.Frame()
	if n := visiblePixels(view, button, card); n == 0 {
		t.Fatal("指针悬停在行上时单条删除按钮应显示")
	}
}

func TestPanelTextsShareOneLeftEdge(t *testing.T) {
	state, _ := newTestSettings()
	state.page = "learned"
	state.wordsLoaded = true
	state.words = []UserWord{{Word: "输入法", Pinyin: "shu ru fa", Freq: 3000}}
	view := ui.NewTester(state.view, 1000, 1100)
	view.Frame()
	row, ok1 := view.Find("学习备份")
	title, ok2 := view.Find("添加词")
	header, ok3 := view.Find("学到的词 1 条")
	if !ok1 || !ok2 || !ok3 {
		t.Fatalf("缺少待核对的文字：行=%v 分组标题=%v 列表标题=%v", ok1, ok2, ok3)
	}
	for name, x := range map[string]float32{"分组标题": title.X, "列表标题": header.X} {
		if math.Abs(float64(x-row.X)) > 1 {
			t.Errorf("%s应与面板内文字左对齐：%.1f / %.1f", name, x, row.X)
		}
	}
}
