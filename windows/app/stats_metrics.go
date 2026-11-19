// Ported from WowType stats_metrics, stats_scope_counts, stats_recent_days and
// stats_format_active at bb2a04fd573f1e5f2444ee68f8f71b7ea429b3c5 (Apache-2.0).
// FreeType adapts its existing daily counters; calendar shifts use AddDate for DST.
package main

import (
	"fmt"
	"slices"
	"time"
)

type typingMetrics struct {
	todayChars, totalChars, todayActive, totalActive      uint64
	todaySpeed, averageSpeed, averagePerDay, fastestSpeed float64
	streak, longestStreak, recordedDays                   int
	bestDay, fastestDay                                   string
	bestChars                                             uint64
}

func typingSpeed(chars, active uint64) float64 {
	if chars == 0 || active == 0 {
		return 0
	}
	return float64(chars) * 60000 / float64(active)
}

func statsMetrics(days map[string]typingDay, now time.Time) typingMetrics {
	today := now.Format(time.DateOnly)
	day := days[today]
	m := typingMetrics{todayChars: sumCounters(day.Characters[:]), todayActive: day.ActiveMS,
		todaySpeed: typingSpeed(day.Characters[0]+day.Characters[1], day.ActiveMS), recordedDays: len(days)}
	var dates []string
	var readable uint64
	for date := range days {
		dates = append(dates, date)
	}
	slices.Sort(dates)
	previous := ""
	run := 0
	for _, date := range dates {
		day := days[date]
		count := sumCounters(day.Characters[:])
		m.totalChars += count
		if day.ActiveMS > 0 {
			m.totalActive += day.ActiveMS
			readable += day.Characters[0] + day.Characters[1]
		}
		if count > m.bestChars {
			m.bestChars, m.bestDay = count, date
		}
		if speed := typingSpeed(day.Characters[0]+day.Characters[1], day.ActiveMS); day.ActiveMS >= 60000 && speed > m.fastestSpeed {
			m.fastestSpeed, m.fastestDay = speed, date
		}
		prev, _ := time.Parse(time.DateOnly, previous)
		if previous != "" && prev.AddDate(0, 0, 1).Format(time.DateOnly) == date {
			run++
		} else {
			run = 1
		}
		m.longestStreak = max(m.longestStreak, run)
		previous = date
	}
	end := now
	if _, ok := days[today]; !ok {
		end = end.AddDate(0, 0, -1)
	}
	for cursor := end; ; cursor = cursor.AddDate(0, 0, -1) {
		if _, ok := days[cursor.Format(time.DateOnly)]; !ok {
			break
		}
		m.streak++
	}
	m.averageSpeed = typingSpeed(readable, m.totalActive)
	if m.recordedDays > 0 {
		m.averagePerDay = float64(m.totalChars) / float64(m.recordedDays)
	}
	return m
}

func statsScopeCounts(days map[string]typingDay) typingDay {
	all := typingDay{Keys: map[int]uint64{}}
	for _, day := range days {
		all.ActiveMS += day.ActiveMS
		for i, n := range day.Characters {
			all.Characters[i] += n
		}
		for i, n := range day.Candidates {
			all.Candidates[i] += n
		}
		for i, n := range day.Hours {
			all.Hours[i] += n
		}
		for scan, n := range day.Keys {
			all.Keys[scan] += n
		}
	}
	return all
}

func statsRecentDays(days map[string]typingDay, now time.Time, length int) ([]string, []uint64) {
	labels := make([]string, length)
	values := make([]uint64, length)
	for i := range labels {
		date := now.AddDate(0, 0, i-length+1)
		labels[i] = date.Format("1月2日")
		day := days[date.Format(time.DateOnly)]
		values[i] = sumCounters(day.Characters[:])
	}
	return labels, values
}

func statsActive(milliseconds uint64) string {
	if milliseconds == 0 {
		return "0 分钟"
	}
	minutes := milliseconds / 60000
	if minutes == 0 {
		return fmt.Sprintf("%d 秒", max(uint64(1), (milliseconds+500)/1000))
	}
	if minutes < 60 {
		return fmt.Sprintf("%d 分钟", minutes)
	}
	if minutes%60 == 0 {
		return fmt.Sprintf("%d 小时", minutes/60)
	}
	return fmt.Sprintf("%d 小时 %d 分钟", minutes/60, minutes%60)
}
