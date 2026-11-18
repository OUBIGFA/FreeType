package main

import (
	"fmt"
	"slices"
	"strconv"
	"time"

	"github.com/egoist/mygo/ui"
)

func (s *settings) loadStats(clear bool) {
	if s.statsBusy {
		return
	}
	s.statsBusy = true
	s.background(func() func() {
		var response Response
		var err error
		if clear {
			_, err = s.backend.Call(op("clear_typing_stats"))
		}
		if err == nil {
			response, err = s.backend.Call(op("list_typing_stats"))
		}
		return func() {
			s.statsBusy = false
			if err != nil {
				s.statsError = err.Error()
				return
			}
			s.stats, s.statsError = response.TypingStats, ""
		}
	})
}

func sumCounters(entries []uint64) (total uint64) {
	for _, count := range entries {
		total += count
	}
	return total
}

func (s *settings) statsPage(c *ui.Context) {
	pageHeader(c, "输入统计", "只在本机保存聚合计数，不记录正文、拼音或应用名称")
	if !s.loaded {
		return
	}
	section(c, "统计设置", func() {
		changed := switchLine(c, "记录输入统计", "", &s.config.TypingStats, false)
		row(c, "保留时长", "", func() {
			days := strconv.Itoa(s.config.StatsRetention)
			choices := []choice{{"0", "永久"}, {"30", "30 天"}, {"90", "90 天"}, {"180", "180 天"}, {"365", "365 天"}}
			if choose(c, "保留时长", choices, &days, 140) {
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
	if ui.AlertDialog(c, &s.confirmStatsClear, "清空输入统计？", "本机的全部输入统计将被删除。", "取消", "清空") == 1 {
		s.loadStats(true)
	}
	if len(s.stats) == 0 {
		ui.Text(c, "暂无统计").Padding(8, rowInset).TextColor(colors(c).muted)
		return
	}

	now := time.Now()
	today := s.stats[now.Format(time.DateOnly)]
	var total, active, readable uint64
	var chars [5]uint64
	var ranks [11]uint64
	var dates []string
	for date, day := range s.stats {
		dates = append(dates, date)
		total += sumCounters(day.Characters[:])
		active += day.ActiveMS
		readable += day.Characters[0] + day.Characters[1]
		for i, n := range day.Characters {
			chars[i] += n
		}
		for i, n := range day.Candidates {
			ranks[i] += n
		}
	}
	streak := 0
	end := now
	if sumCounters(today.Characters[:]) == 0 {
		end = end.AddDate(0, 0, -1)
	}
	for day := end; sumCounters(s.stats[day.Format(time.DateOnly)].CharactersSlice()) > 0; day = day.AddDate(0, 0, -1) {
		streak++
	}
	speed := "—"
	if active > 0 {
		speed = fmt.Sprintf("%.0f 字/分钟", float64(readable)*60000/float64(active))
	}
	section(c, "概览", func() {
		for _, item := range [][2]string{
			{"今日字数", fmt.Sprint(sumCounters(today.Characters[:]))}, {"累计字数", fmt.Sprint(total)},
			{"活跃时长", fmt.Sprintf("%.1f 分钟", float64(active)/60000)}, {"连续输入", fmt.Sprintf("%d 天", streak)}, {"平均打字速度", speed},
		} {
			row(c, item[0], "", func() { ui.Text(c, item[1]) })
		}
	})
	section(c, "近 12 个月", func() { s.statsCalendar(c, now) })
	section(c, "今日时段", func() { statsBars(c, today.Hours[:], func(i int) string { return fmt.Sprintf("%02d:00", i) }) })
	section(c, "字符分类", func() {
		statsBars(c, chars[:], func(i int) string { return []string{"汉字", "英文", "数字", "标点", "其他"}[i] })
	})
	section(c, "候选位次", func() {
		statsBars(c, ranks[:], func(i int) string {
			if i == 10 {
				return "11+"
			}
			return strconv.Itoa(i + 1)
		})
	})
	section(c, "今日物理键位", func() { statsKeyboard(c, today.Keys) })
	slices.Sort(dates)
	slices.Reverse(dates)
	section(c, "近 30 天明细", func() {
		cutoff := now.AddDate(0, 0, -29).Format(time.DateOnly)
		for _, date := range dates {
			if date < cutoff || date > now.Format(time.DateOnly) {
				continue
			}
			day := s.stats[date]
			row(c, date, "", func() {
				ui.Text(c, fmt.Sprintf("%d 字 · %.1f 分钟", sumCounters(day.Characters[:]), float64(day.ActiveMS)/60000))
			})
		}
	})
}

func (day typingDay) CharactersSlice() []uint64 { return day.Characters[:] }

func statsBars(c *ui.Context, values []uint64, label func(int) string) {
	var peak uint64 = 1
	for _, value := range values {
		peak = max(peak, value)
	}
	ui.Column(c).FillWidth().Padding(8, rowInset).Gap(4).Children(func() {
		for i, value := range values {
			ui.Row(c).FillWidth().AlignItems(ui.Center).Gap(8).Children(func() {
				ui.Text(c, label(i)).Width(52).Shrink(0)
				ui.Column(c).Grow(1).Height(8).Radius(3).Background(colors(c).subtle).Children(func() {
					if value > 0 {
						ui.Box(c).WidthPercent(100 * float32(value) / float32(peak)).Height(8).Radius(3).Background(brandPink)
					}
				})
				ui.Text(c, fmt.Sprint(value)).Width(58).TextAlign(ui.End)
			})
		}
	})
}

func heatColor(c *ui.Context, count uint64) ui.Color {
	if count == 0 {
		return colors(c).subtle
	}
	switch {
	case count < 10:
		return ui.Hex("#c58fc2")
	case count < 100:
		return ui.Hex("#b462ae")
	default:
		return ui.Hex("#953a8d")
	}
}

func (s *settings) statsCalendar(c *ui.Context, now time.Time) {
	start := now.AddDate(-1, 0, 1)
	for start.Weekday() != time.Monday {
		start = start.AddDate(0, 0, -1)
	}
	ui.ScrollHorizontal(c).FillWidth().Padding(8, rowInset).Children(func() {
		ui.Row(c).Shrink(0).Gap(3).Children(func() {
			for week := start; !week.After(now); week = week.AddDate(0, 0, 7) {
				ui.Column(c).Gap(3).Children(func() {
					for weekday := 0; weekday < 7; weekday++ {
						date := week.AddDate(0, 0, weekday).Format(time.DateOnly)
						count := sumCounters(s.stats[date].CharactersSlice())
						ui.Box(c).Size(11, 11).Radius(2).Background(heatColor(c, count)).Tooltip(fmt.Sprintf("%s：%d 字", date, count))
					}
				})
			}
		})
	})
}

func statsKeyboard(c *ui.Context, keys map[int]uint64) {
	// Set-1 scan positions retain physical row order independent of active Windows layout.
	rows := []struct {
		start  int
		labels string
	}{{0x02, "1234567890-="}, {0x10, "QWERTYUIOP[]"}, {0x1e, "ASDFGHJKL;'"}, {0x2c, "ZXCVBNM,./"}}
	ui.ScrollHorizontal(c).FillWidth().Padding(8, rowInset).Children(func() {
		ui.Column(c).Shrink(0).Gap(4).Children(func() {
			for _, row := range rows {
				ui.Row(c).Gap(4).Children(func() {
					for offset, key := range row.labels {
						count := keys[row.start+offset]
						fg := c.Theme().Text
						if count > 0 {
							fg = ui.Hex("#18181b")
							if count >= 100 {
								fg = ui.Hex("#ffffff")
							}
						}
						ui.Text(c, string(key)).Size(23, 24).Center().TextColor(fg).Background(heatColor(c, count)).Radius(3).Tooltip(fmt.Sprint(count))
					}
				})
			}
		})
	})
}
