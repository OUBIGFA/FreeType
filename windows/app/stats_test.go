package main

import (
	"fmt"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

func TestPhysicalKeyboardKeepsLongKeysAndExtendedKeyCounts(t *testing.T) {
	keys := map[int]uint64{0x1e: 12, 0x39: 80, 0x0e: 4, 0x1c: 7, 0x11c: 3, 0x14b: 2}
	for _, dark := range []bool{false, true} {
		view := ui.NewTester(func(c *ui.Context) {
			ui.Column(c).Fill().Background(colors(c).card).Children(func() { statsKeyboard(c, keys) })
		}, 720, 560)
		view.SetDark(dark)
		view.Frame()
		for _, label := range []string{"Esc", "F12", "Tab", "Caps Lock", "Backspace", "Enter", "Space", "Shift", "Ctrl"} {
			if !view.HasText(label) {
				t.Errorf("physical keyboard is missing %s", label)
			}
		}
		letter, ok := view.Find("键位：A")
		space, spaceOK := view.Find("键位：Space")
		if !ok || !spaceOK || space.W < letter.W*5 || space.Y <= letter.Y {
			t.Fatalf("keyboard lost the physical size or position of Space: letter=%+v space=%+v", letter, space)
		}
		for _, text := range []string{"共 108 次按键 · 6 个键位", "小键盘回车", "左箭头", "3 次", "2 次"} {
			if !view.HasText(text) {
				t.Errorf("missing count or extended key: %s", text)
			}
		}
		saveUIFrame(t, fmt.Sprintf("physical-keyboard-dark-%v", dark), view)
	}
}

func TestStatsViewsReuseUpstreamOverviewAndBreakdowns(t *testing.T) {
	s, _ := newTestSettings()
	s.resources = "resources"
	s.page, s.config.TypingStats = "stats", true
	now := time.Now()
	s.stats = map[string]typingDay{
		now.Format(time.DateOnly): {Characters: [5]uint64{120, 30, 10, 5, 2}, ActiveMS: 120000,
			Keys: map[int]uint64{0x1e: 12, 0x39: 80}, Candidates: [11]uint64{8, 2}, Hours: [24]uint64{6: 167}},
		now.AddDate(0, 0, -1).Format(time.DateOnly): {Characters: [5]uint64{60}, ActiveMS: 60000},
	}
	view := ui.NewTester(s.view, 1000, 1800)
	for _, label := range []string{"今日输入", "累计输入", "今日活跃", "今日速度", "平均速度", "连续天数"} {
		if !view.HasText(label) {
			t.Fatalf("missing upstream overview metric: %s", label)
		}
	}
	for _, item := range []struct{ tab, expected string }{{"按键", "今日物理键位"}, {"趋势", "每日趋势 · 近 30 天"}, {"类型", "字符类型"}, {"候选", "候选命中位置"}} {
		if err := view.Click(item.tab); err != nil {
			t.Fatal(err)
		}
		view.Frame()
		if !view.HasText(item.expected) {
			t.Fatalf("missing upstream breakdown: %s", item.expected)
		}
		saveUIFrame(t, "stats-upstream-"+item.tab, view)
	}
	s.statsTab = 0
	for _, dark := range []bool{false, true} {
		for _, scale := range []float32{1, 1.5} {
			view.SetSize(780, 1600)
			view.SetDark(dark)
			view.SetPreferences(ui.Preferences{TextScale: scale})
			view.Frame()
			for _, label := range []string{"今日输入", "累计输入", "今日活跃", "今日速度", "平均速度", "连续天数"} {
				bounds, ok := view.Find(label)
				if !ok || bounds.X < 0 || bounds.X+bounds.W > 780 {
					t.Fatalf("statistics metric clipped at text scale %v: %s %+v", scale, label, bounds)
				}
			}
			saveUIFrame(t, fmt.Sprintf("stats-minimum-dark-%v-scale-%.1f", dark, scale), view)
		}
	}
}

func TestUpstreamMetricsKeepInactiveDaysOutOfSpeedAndFillTrendGaps(t *testing.T) {
	now := time.Date(2026, 11, 20, 12, 0, 0, 0, time.Local)
	days := map[string]typingDay{
		"2026-11-18": {Characters: [5]uint64{40, 20}, ActiveMS: 60000},
		"2026-11-19": {Characters: [5]uint64{600}, ActiveMS: 0},
	}
	m := statsMetrics(days, now)
	if m.totalChars != 660 || m.averageSpeed != 60 || m.averagePerDay != 330 || m.streak != 2 || m.longestStreak != 2 || m.fastestDay != "2026-11-18" {
		t.Fatalf("statistics diverged from upstream's speed and calendar rules: %+v", m)
	}
	_, values := statsRecentDays(days, now, 4)
	if fmt.Sprint(values) != "[0 60 600 0]" {
		t.Fatalf("missing dates must stay zero: %v", values)
	}
	for _, sample := range []struct {
		count, max uint64
		want       int
	}{{0, 100, 0}, {1, 100, 1}, {25, 100, 1}, {26, 100, 2}, {75, 100, 3}, {76, 100, 4}, {1, 3, 2}, {2, 3, 3}} {
		if got := keyboardHeatLevel(sample.count, sample.max); got != sample.want {
			t.Errorf("heat(%d,%d)=%d, want %d", sample.count, sample.max, got, sample.want)
		}
	}
}

func TestPhysicalKeyboardEmptyAndLargeTextRemainReadable(t *testing.T) {
	for _, width := range []int{448, 688} {
		view := ui.NewTester(func(c *ui.Context) { statsKeyboard(c, nil) }, width, 680)
		view.SetPreferences(ui.Preferences{TextScale: 1.5})
		view.Frame()
		if !view.HasText("暂无按键记录") || !view.HasText("Space") {
			t.Fatal("an empty day must keep the keyboard and explain the zero counts")
		}
		saveUIFrame(t, fmt.Sprintf("physical-keyboard-empty-%d", width), view)
	}
}
