// Native port of WowType's overview, keyboard/trend/kind/rank tabs at
// bb2a04fd573f1e5f2444ee68f8f71b7ea429b3c5, distributed under Apache-2.0.
// Use mygo's standard grid, progress and chart controls in place of GTK/Cairo.
package main

import (
	"fmt"
	"math"
	"slices"
	"strconv"
	"strings"
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
			ui.Column(c).FillWidth().Padding(14, 0).Gap(6).Children(func() {
				ui.Text(c, "关闭时不新增计数；开启后在本机按天记录字符、按键、活跃时长和候选位置").LineHeight(1.45).Wrap()
				if len(s.stats) > 0 {
					ui.Text(c, fmt.Sprintf("已有 %d 天的记录保留；可在上方清空", len(s.stats))).FontSize(textSize(c, 13)).TextColor(colors(c).muted).LineHeight(1.45).Wrap()
				}
			})
		})
		return
	}
	now := time.Now()
	s.statsOverview(c, statsMetrics(s.stats, now), now)
	hint(c, "速度只统计汉字与字母；两次上屏间隔超过 10 秒视为休息，不计入活跃时长")
	ui.Row(c).FillWidth().Justify(ui.SpaceBetween).AlignItems(ui.Center).Wrap().Gap(10).Margin(18, 0, 0, 0).Children(func() {
		ui.Segmented(c, &s.statsTab, "按键", "趋势", "类型", "候选").Label("统计分类")
		if s.statsTab == 0 {
			ui.Segmented(c, &s.statsScope, "今日", "累计").Label("按键统计范围")
		}
	})
	all := statsScopeCounts(s.stats)
	switch s.statsTab {
	case 0:
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
				ui.Text(c, "这 30 天还没有输入记录").FontSize(textSize(c, 13)).TextColor(colors(c).muted).Padding(14, 0)
			}
			for _, date := range dates {
				day := s.stats[date]
				ui.Row(c).FillWidth().Padding(10, 0).MinHeight(46).Gap(16).AlignItems(ui.Center).Children(func() {
					ui.Text(c, date).FontWeight(500).FontFeatures("tnum").Grow(1)
					ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
						ui.Text(c, fmt.Sprintf("%d 字符", sumCounters(day.Characters[:]))).FontWeight(500).FontFeatures("tnum")
						ui.Text(c, "·").TextColor(colors(c).muted)
						ui.Text(c, statsActive(day.ActiveMS)).FontSize(textSize(c, 13)).TextColor(colors(c).muted).FontFeatures("tnum")
					})
				})
			}
		})
	case 2:
		section(c, "字符类型", func() { statsKinds(c, all.Characters) })
	case 3:
		section(c, "候选命中位置", func() { statsRanks(c, all.Candidates) })
	}
}

func statsActiveParts(milliseconds uint64) [][2]string {
	if milliseconds == 0 {
		return [][2]string{{"0", "分钟"}}
	}
	minutes := milliseconds / 60000
	if minutes == 0 {
		return [][2]string{{fmt.Sprint(max(uint64(1), (milliseconds+500)/1000)), "秒"}}
	}
	if minutes < 60 {
		return [][2]string{{fmt.Sprint(minutes), "分钟"}}
	}
	if minutes%60 == 0 {
		return [][2]string{{fmt.Sprint(minutes / 60), "小时"}}
	}
	return [][2]string{{fmt.Sprint(minutes / 60), "小时"}, {fmt.Sprint(minutes % 60), "分钟"}}
}

func formatCount(n uint64) string {
	s := strconv.FormatUint(n, 10)
	if len(s) <= 3 {
		return s
	}
	var b strings.Builder
	b.Grow(len(s) + (len(s)-1)/3)
	rem := len(s) % 3
	if rem == 0 {
		rem = 3
	}
	b.WriteString(s[:rem])
	for i := rem; i < len(s); i += 3 {
		b.WriteByte(',')
		b.WriteString(s[i : i+3])
	}
	return b.String()
}

func statsDayLabel(date string, now time.Time) string {
	parsed, err := time.Parse(time.DateOnly, date)
	if err != nil {
		return date
	}
	if parsed.Year() == now.Year() {
		return parsed.Format("1月2日")
	}
	return parsed.Format(time.DateOnly)
}

func (s *settings) statsOverview(c *ui.Context, m typingMetrics, now time.Time) {
	p := colors(c)
	type overviewItem struct {
		title, valueLabel string
		parts             [][2]string
		notes             []string
	}
	var bestNotes []string
	if m.bestDay != "" {
		bestNotes = []string{fmt.Sprintf("最高 %s", formatCount(m.bestChars)), statsDayLabel(m.bestDay, now)}
	}
	avgNotes := []string{"暂无记录"}
	if m.recordedDays > 0 {
		avgNotes = []string{
			fmt.Sprintf("日均 %s", formatCount(uint64(math.Round(max(0, m.averagePerDay))))),
			fmt.Sprintf("%d 天有记录", m.recordedDays),
		}
	}
	var activeNotes []string
	if m.totalActive > 0 {
		activeNotes = []string{"累计 " + statsActive(m.totalActive)}
	}
	var fastestNotes []string
	if m.fastestDay != "" {
		fastestNotes = []string{
			fmt.Sprintf("最快 %s", formatCount(uint64(math.Round(max(0, m.fastestSpeed))))),
			statsDayLabel(m.fastestDay, now),
		}
	}
	rows := [][]overviewItem{
		{
			{title: "今日输入", parts: [][2]string{{formatCount(m.todayChars), "字符"}}, notes: bestNotes},
			{title: "累计输入", parts: [][2]string{{formatCount(m.totalChars), "字符"}}, notes: avgNotes},
			{title: "今日活跃", valueLabel: statsActive(m.todayActive), parts: statsActiveParts(m.todayActive), notes: activeNotes},
		},
		{
			{title: "今日速度", parts: [][2]string{{formatCount(uint64(math.Round(max(0, m.todaySpeed)))), "字/分钟"}}, notes: fastestNotes},
			{title: "平均速度", parts: [][2]string{{formatCount(uint64(math.Round(max(0, m.averageSpeed)))), "字/分钟"}}},
			{title: "连续天数", parts: [][2]string{{formatCount(uint64(max(0, m.streak))), "天"}}, notes: []string{fmt.Sprintf("最长 %d 天", m.longestStreak)}},
		},
	}
	section(c, "概览", func() {
		for rowIndex, items := range rows {
			rowBox := ui.Row(c).FillWidth().AlignItems(ui.Stretch).Gap(25).Dividers(1, p.subBorder)
			if rowIndex == 0 {
				rowBox.Padding(14, 0, 16, 0)
			} else {
				rowBox.Padding(16, 0, 14, 0)
			}
			rowBox.Children(func() {
				for _, item := range items {
					ui.Column(c).Grow(1).Basis(0).MinWidth(0).Gap(6).Children(func() {
						ui.Text(c, item.title).FontSize(textSize(c, 13)).FontWeight(500).TextColor(p.muted)
						valueRow := ui.Row(c).Wrap().Gap(4).AlignItems(ui.End)
						if item.valueLabel != "" {
							valueRow.Label(item.valueLabel)
						}
						valueRow.Children(func() {
							for _, part := range item.parts {
								ui.Text(c, part[0]).FontSize(textSize(c, 26)).FontWeight(600).FontFeatures("tnum").Wrap()
								if part[1] != "" {
									ui.Text(c, part[1]).FontSize(textSize(c, 13)).TextColor(p.muted).Padding(0, 0, 3, 0)
								}
							}
						})
						if len(item.notes) > 0 {
							ui.Row(c).FillWidth().Wrap().Gap(8).AlignItems(ui.Center).Children(func() {
								for _, note := range item.notes {
									ui.Text(c, note).FontSize(textSize(c, 12.5)).TextColor(p.muted).FontFeatures("tnum").LineHeight(1.35).Wrap()
								}
							})
						}
					})
				}
			})
		}
	})
}

// Upstream bar proportions and sparse end labels, rendered by the native chart library.
func statsTrend(c *ui.Context, labels []string, values []uint64, unit string) {
	p := colors(c)
	peak := slices.Max(values)
	ui.Column(c).Padding(14, 0).Gap(10).Children(func() {
		ui.Row(c).FillWidth().Justify(ui.SpaceBetween).AlignItems(ui.Center).Wrap().Gap(8).Children(func() {
			ui.Text(c, fmt.Sprintf("共 %d %s · 最高 %d %s", sumCounters(values), unit, peak, unit)).
				FontSize(textSize(c, 13)).TextColor(p.muted).FontFeatures("tnum").Wrap()
			if peak == 0 {
				ui.Text(c, "此时段暂无输入记录").FontSize(textSize(c, 12.5)).TextColor(p.muted)
			}
		})
		x := chart.NewBand(labels...).Padding(.15, 0)
		y := chart.NewLinear(0, float64(max(peak, 1)))
		chart.Plot(c, chart.Axes{}, func(area ui.Element) {
			area.BorderWidth(0, 0, 1, 0).BorderColor(p.border)
			for i, value := range values {
				if value > 0 {
					chart.Rect(c, x.Band(i), y.Span(0, float64(value))).Background(ui.Hex("#0eaf76")).Radius(3, 3, 0, 0).
						Tooltip(fmt.Sprintf("%s：%d %s", labels[i], value, unit)).Label(labels[i])
				}
			}
		}).Height(150).FillWidth().Label("输入趋势图")
		ui.Row(c).FillWidth().Justify(ui.SpaceBetween).Children(func() {
			ui.Text(c, labels[0]).FontSize(textSize(c, 12)).TextColor(p.muted).FontFeatures("tnum")
			ui.Text(c, labels[len(labels)-1]).FontSize(textSize(c, 12)).TextColor(p.muted).FontFeatures("tnum")
		})
	})
}

func statsKinds(c *ui.Context, values [5]uint64) {
	p := colors(c)
	names := []string{"汉字", "字母", "数字", "标点", "其他（含表情）"}
	shades := []ui.Color{ui.Hex("#0eaf76"), ui.Hex("#4c9aff"), ui.Hex("#f5a623"), ui.Hex("#8a93a6"), ui.Hex("#c4c9c7")}
	total := sumCounters(values[:])
	ui.Column(c).Padding(14, 0).Gap(14).Children(func() {
		if total == 0 {
			ui.Text(c, "还没有字符记录").FontSize(textSize(c, 13)).TextColor(p.muted)
			return
		}
		ui.Column(c).FillWidth().Radius(6).Clip().Children(func() {
			chart.Plot(c, chart.Axes{}, func(_ ui.Element) {
				var offset float32
				for i, value := range values {
					share := float32(float64(value) / float64(total))
					chart.Rect(c, chart.Span{Lo: offset, Hi: offset + share}, chart.Span{Lo: 0, Hi: 1}).Background(shades[i])
					offset += share
				}
			}).Height(12).FillWidth().Label("字符类型占比")
		})
		ui.Column(c).FillWidth().Gap(1).Dividers(1, p.subBorder).Children(func() {
			for i, value := range values {
				if value == 0 {
					continue
				}
				ui.Row(c).FillWidth().Padding(8, 0).Gap(10).AlignItems(ui.Center).Children(func() {
					ui.Box(c).Size(10, 10).Radius(3).Background(shades[i])
					ui.Text(c, names[i]).FontWeight(500).Grow(1)
					ui.Text(c, fmt.Sprintf("%d", value)).FontSize(textSize(c, 13)).TextColor(p.muted).FontFeatures("tnum")
					ui.Text(c, fmt.Sprintf("%.0f%%", 100*float64(value)/float64(total))).FontWeight(500).FontFeatures("tnum").
						Width(textSize(c, 44)).TextAlign(ui.End)
				})
			}
		})
	})
}

func statsRanks(c *ui.Context, values [11]uint64) {
	p := colors(c)
	total := sumCounters(values[:])
	peak := slices.Max(values[:])
	ui.Column(c).Padding(14, 0).Gap(10).Children(func() {
		if total == 0 {
			ui.Text(c, "还没有候选记录；选词后可查看命中位置").FontSize(textSize(c, 13)).Wrap().TextColor(p.muted)
			return
		}
		ui.Row(c).FillWidth().Justify(ui.SpaceBetween).AlignItems(ui.Center).Wrap().Gap(8).Padding(0, 0, 4, 0).Children(func() {
			ui.Text(c, fmt.Sprintf("首选命中 %.0f%%", 100*float64(values[0])/float64(total))).
				FontSize(textSize(c, 15)).FontWeight(600).FontFeatures("tnum")
			ui.Text(c, fmt.Sprintf("共 %d 次上屏", total)).
				FontSize(textSize(c, 13)).TextColor(p.muted).FontFeatures("tnum")
		})
		for i, value := range values {
			if i == 10 && value == 0 {
				continue
			}
			label := fmt.Sprintf("第 %d 个", i+1)
			if i == 10 {
				label = "第 11 个及以后"
			}
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				nameText := ui.Text(c, label).FontSize(textSize(c, 13)).Width(textSize(c, 96)).Wrap()
				if value == 0 {
					nameText.TextColor(p.muted)
				}
				ui.Progress(c, float64(value)/float64(peak)).Grow(1).MinWidth(0).Label(label + "命中占比")
				countText := ui.Text(c, fmt.Sprintf("%d 次", value)).FontSize(textSize(c, 13)).FontFeatures("tnum").
					Width(textSize(c, 60)).TextAlign(ui.End)
				if value == 0 {
					countText.TextColor(p.muted)
				}
			})
		}
	})
}

// Retain FreeType's yearly history using native chart cells and upstream heat levels.
func (s *settings) statsCalendar(c *ui.Context, now time.Time) {
	p := colors(c)
	start := now.AddDate(-1, 0, 1)
	for start.Weekday() != time.Monday {
		start = start.AddDate(0, 0, -1)
	}
	var peak uint64
	for _, day := range s.stats {
		peak = max(peak, sumCounters(day.Characters[:]))
	}
	ui.Column(c).FillWidth().Padding(14, 0).Gap(10).Children(func() {
		ui.ScrollHorizontal(c).FillWidth().Children(func() {
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
		ui.Row(c).FillWidth().Justify(ui.SpaceBetween).AlignItems(ui.Center).Wrap().Gap(8).Children(func() {
			ui.Text(c, start.Format("2006-01")+" 至 "+now.Format("2006-01")).FontSize(textSize(c, 12)).TextColor(p.muted).FontFeatures("tnum")
			ui.Row(c).Gap(4).AlignItems(ui.Center).Children(func() {
				ui.Text(c, "少").FontSize(textSize(c, 12)).TextColor(p.muted)
				for level := 0; level < 5; level++ {
					bg, _ := keyboardHeatColors(c, level)
					ui.Box(c).Size(12, 12).Radius(2).Background(bg)
				}
				ui.Text(c, "多").FontSize(textSize(c, 12)).TextColor(p.muted)
			})
		})
	})
}
