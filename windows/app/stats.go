package main

import (
	"time"
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
			s.statsUpdatedAt = time.Now()
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
