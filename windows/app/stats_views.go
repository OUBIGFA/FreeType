// Native port of WowType's overview, keyboard/trend/kind/rank tabs at
// bb2a04fd573f1e5f2444ee68f8f71b7ea429b3c5, distributed under Apache-2.0.
// Use mygo's standard grid, progress and chart controls in place of GTK/Cairo.
package main

import (
	"fmt"
	"slices"
	"strconv"
	"time"

	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/chart"
)

func (s *settings) statsPage(c *ui.Context) {
	pageHeader(c, "输入统计", "只在这台电脑保存统计计数；不记录正文、拼音或应用名称")
	if !s.loaded {
		return
	}
	// Upstream checks for changes every two seconds while this page is visible.
	// Read the server's in-memory snapshot, including counts not yet written to disk.
	if !s.inline {
		if time.Since(s.statsUpdatedAt) >= 2*time.Second {
			s.loadStats(false)
		}
		c.After(2 * time.Second)
	}
	section(c, "统计设置", func() {
		changed := switchLine(c, "记录输入统计", "默认关闭；关闭后保留已有计数", &s.config.TypingStats, false)
		row(c, "保留时长", "", func() {
			days := strconv.Itoa(s.config.StatsRetention)
			if choose(c, "保留时长", []choice{{"0", "永久"}, {"30", "30 天"}, {"90", "90 天"}, {"180", "180 天"}, {"365", "365 天"}}, &days, 140) {
				s.config.StatsRetention, _ = strconv.Atoi(days)
				changed = true
			}
		})
		row(c, "本机数据", s.statsError, func() {
			if ui.Button(c, "刷新").Disabled(s.statsBusy).Clicked() {
				s.loadStats(false)
			}
			if ui.Button(c, "清空").Disabled(s.statsBusy || (len(s.stats) == 0 && s.statsError == "")).Clicked() {
				s.confirmStatsClear = true
			}
		})
		if changed {
			s.saveConfig()
		}
	})
	if ui.AlertDialog(c, &s.confirmStatsClear, "清空输入统计", "删除这台电脑的全部输入统计", "取消", "清空") == 1 {
		s.loadStats(true)
	}
	if !s.config.TypingStats {
		section(c, "输入统计已关闭", func() {
			ui.Text(c, "关闭时不新增计数；开启后在本机按天记录字符、按键、活跃时长和候选位置").Padding(12, 0).Wrap()
			if len(s.stats) > 0 {
				ui.Text(c, fmt.Sprintf("已有 %d 天的记录保留；可在上方清空", len(s.stats))).Padding(0, 0, 12, 0).Wrap()
			}
		})
		return
	}
	now := time.Now()
	s.statsOverview(c, statsMetrics(s.stats, now))
	hint(c, "速度只统计汉字与字母；两次上屏间隔超过 10 秒视为休息，不计入活跃时长")
	ui.Segmented(c, &s.statsTab, "按键", "趋势", "类型", "候选").Label("统计分类").AlignSelf(ui.Start).Margin(18, 0, 0, 0)
	all := statsScopeCounts(s.stats)
	switch s.statsTab {
	case 0:
		ui.Segmented(c, &s.statsScope, "今日", "累计").Label("按键统计范围").AlignSelf(ui.Start).Margin(12, 0, 0, 0)
		keys, title := s.stats[now.Format(time.DateOnly)].Keys, "今日物理键位"
		if s.statsScope == 1 {
			keys, title = all.Keys, "累计物理键位"
		}
		section(c, title, func() { statsKeyboard(c, keys) })
	case 1:
		labels, values := statsRecentDays(s.stats, now, 30)
		section(c, "每日趋势 · 近 30 天", func() { statsTrend(c, labels, values, "字符") })
		day := s.stats[now.Format(time.DateOnly)]
		hours := make([]string, 24)
		for i := range hours {
			hours[i] = fmt.Sprintf("%02d:00", i)
		}
		section(c, "今日时段", func() { statsTrend(c, hours, day.Hours[:], "字符") })
		section(c, "近 12 个月", func() { s.statsCalendar(c, now) })
		section(c, "近 30 天明细", func() {
			var dates []string
			cutoff := now.AddDate(0, 0, -29).Format(time.DateOnly)
			for date := range s.stats {
				if date >= cutoff && date <= now.Format(time.DateOnly) {
					dates = append(dates, date)
				}
			}
			slices.Sort(dates)
			slices.Reverse(dates)
			if len(dates) == 0 {
				ui.Text(c, "这 30 天还没有输入记录").Padding(14, 0)
			}
			for _, date := range dates {
				day := s.stats[date]
				row(c, date, "", func() {
					ui.Text(c, fmt.Sprintf("%d 字符 · %s", sumCounters(day.Characters[:]), statsActive(day.ActiveMS)))
				})
			}
		})
	case 2:
		section(c, "字符类型", func() { statsKinds(c, all.Characters) })
	case 3:
		section(c, "候选命中位置", func() { statsRanks(c, all.Candidates) })
	}
}

func (s *settings) statsOverview(c *ui.Context, m typingMetrics) {
	section(c, "概览", func() {
		// 按各列内容定宽，把剩余宽度分给列间距，避免短列后面留下大片空白。
		ui.Grid(c).FillWidth().ColumnTracks(ui.FitContent(), ui.FitContent(), ui.FitContent()).Justify(ui.SpaceBetween).GapX(24).GapY(16).Padding(14, 0).Children(func() {
			best := ""
			if m.bestDay != "" {
				best = fmt.Sprintf("单日最多 %d 字符 · %s", m.bestChars, m.bestDay)
			}
			fastest := ""
			if m.fastestDay != "" {
				fastest = fmt.Sprintf("单日最快 %.0f 字/分钟 · %s", m.fastestSpeed, m.fastestDay)
			}
			items := [][4]string{
				{"今日输入", fmt.Sprint(m.todayChars), "字符", best},
				{"累计输入", fmt.Sprint(m.totalChars), "字符", fmt.Sprintf("日均 %.0f 字符 · %d 天有记录", m.averagePerDay, m.recordedDays)},
				{"今日活跃", statsActive(m.todayActive), "", "累计 " + statsActive(m.totalActive)},
				{"今日速度", fmt.Sprintf("%.0f", m.todaySpeed), "字/分钟", fastest},
				{"平均速度", fmt.Sprintf("%.0f", m.averageSpeed), "字/分钟", ""},
				{"连续天数", fmt.Sprint(m.streak), "天", fmt.Sprintf("最长 %d 天", m.longestStreak)},
			}
			for _, item := range items {
				ui.Column(c).Gap(4).MinWidth(0).Children(func() {
					ui.Text(c, item[0]).FontSize(textSize(c, 13)).TextColor(colors(c).muted)
					ui.Row(c).Wrap().Gap(4).AlignItems(ui.End).Children(func() {
						ui.Text(c, item[1]).FontSize(textSize(c, 26)).FontWeight(600).Wrap()
						if item[2] != "" {
							ui.Text(c, item[2]).FontSize(textSize(c, 12)).TextColor(colors(c).muted).Padding(0, 0, 3, 0)
						}
					})
					if item[3] != "" {
						ui.Text(c, item[3]).FontSize(textSize(c, 12)).TextColor(colors(c).muted).Wrap()
					}
				})
			}
		})
	})
}

// Upstream bar proportions and sparse end labels, rendered by the native chart library.
func statsTrend(c *ui.Context, labels []string, values []uint64, unit string) {
	peak := slices.Max(values)
	ui.Column(c).Padding(14, 0).Gap(12).Children(func() {
		if peak == 0 {
			ui.Text(c, "此时段暂无输入记录").TextColor(colors(c).muted)
		}
		x := chart.NewBand(labels...).Padding(.15, 0)
		y := chart.NewLinear(0, float64(max(peak, 1)))
		chart.Plot(c, chart.Axes{}, func(area ui.Element) {
			area.BorderWidth(0, 0, 1, 0).BorderColor(colors(c).border)
			for i, value := range values {
				if value > 0 {
					chart.Rect(c, x.Band(i), y.Span(0, float64(value))).Background(ui.Hex("#0eaf76")).
						Tooltip(fmt.Sprintf("%s：%d %s", labels[i], value, unit)).Label(labels[i])
				}
			}
		}).Height(150).FillWidth().Label("输入趋势图")
		ui.Row(c).FillWidth().Justify(ui.SpaceBetween).Children(func() { ui.Text(c, labels[0]); ui.Text(c, labels[len(labels)-1]) })
		ui.Text(c, fmt.Sprintf("共 %d %s · 最高 %d %s", sumCounters(values), unit, peak, unit)).TextColor(colors(c).muted).Wrap()
	})
}

func statsKinds(c *ui.Context, values [5]uint64) {
	names := []string{"汉字", "字母", "数字", "标点", "其他（含表情）"}
	shades := []ui.Color{ui.Hex("#0eaf76"), ui.Hex("#4c9aff"), ui.Hex("#f5a623"), ui.Hex("#8a93a6"), ui.Hex("#c4c9c7")}
	total := sumCounters(values[:])
	ui.Column(c).Padding(14, 0).Gap(12).Children(func() {
		if total == 0 {
			ui.Text(c, "还没有字符记录").TextColor(colors(c).muted)
			return
		}
		chart.Plot(c, chart.Axes{}, func(_ ui.Element) {
			var offset float32
			for i, value := range values {
				share := float32(float64(value) / float64(total))
				chart.Rect(c, chart.Span{Lo: offset, Hi: offset + share}, chart.Span{Lo: 0, Hi: 1}).Background(shades[i])
				offset += share
			}
		}).Height(14).FillWidth().Label("字符类型占比")
		for i, value := range values {
			if value == 0 {
				continue
			}
			ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Box(c).Size(10, 10).Background(shades[i])
				ui.Text(c, names[i]).Grow(1)
				ui.Text(c, fmt.Sprintf("%d · %.0f%%", value, 100*float64(value)/float64(total)))
			})
		}
	})
}

func statsRanks(c *ui.Context, values [11]uint64) {
	total := sumCounters(values[:])
	peak := slices.Max(values[:])
	ui.Column(c).Padding(14, 0).Gap(10).Children(func() {
		if total == 0 {
			ui.Text(c, "还没有候选记录；选词后可查看命中位置").Wrap().TextColor(colors(c).muted)
			return
		}
		ui.Text(c, fmt.Sprintf("首选命中 %.0f%% · 共 %d 次上屏", 100*float64(values[0])/float64(total), total)).Wrap()
		for i, value := range values {
			if i == 10 && value == 0 {
				continue
			}
			label := fmt.Sprintf("第 %d 个", i+1)
			if i == 10 {
				label = "第 11 个及以后"
			}
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				ui.Text(c, label).Width(textSize(c, 96)).Wrap()
				ui.Progress(c, float64(value)/float64(peak)).Grow(1).MinWidth(0).Label(label + "命中占比")
				ui.Text(c, fmt.Sprintf("%d 次", value)).Width(textSize(c, 60)).TextAlign(ui.End)
			})
		}
	})
}

// Retain FreeType's yearly history using native chart cells and upstream heat levels.
func (s *settings) statsCalendar(c *ui.Context, now time.Time) {
	start := now.AddDate(-1, 0, 1)
	for start.Weekday() != time.Monday {
		start = start.AddDate(0, 0, -1)
	}
	var peak uint64
	for _, day := range s.stats {
		peak = max(peak, sumCounters(day.Characters[:]))
	}
	ui.ScrollHorizontal(c).FillWidth().Padding(14, 0).Children(func() {
		chart.Plot(c, chart.Axes{}, func(_ ui.Element) {
			for week, date := 0, start; !date.After(now); week, date = week+1, date.AddDate(0, 0, 7) {
				for weekday := 0; weekday < 7; weekday++ {
					at := date.AddDate(0, 0, weekday)
					if at.After(now) {
						continue
					}
					day := s.stats[at.Format(time.DateOnly)]
					count := sumCounters(day.Characters[:])
					bg, _ := keyboardHeatColors(c, keyboardHeatLevel(count, peak))
					chart.Rect(c, chart.Span{Lo: float32(week) / 54, Hi: (float32(week) + .8) / 54}, chart.Span{Lo: float32(6-weekday) / 7, Hi: (float32(6-weekday) + .8) / 7}).Background(bg).Radius(2).
						Tooltip(fmt.Sprintf("%s：%d 字符", at.Format(time.DateOnly), count))
				}
			}
		}).Size(650, 90).Shrink(0).Label("年度输入记录")
	})
}
