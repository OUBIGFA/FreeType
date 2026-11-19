package main

import (
	"testing"
	"time"
)

type delayedConfigBackend struct {
	requests chan Config
	release  chan struct{}
}

func (b *delayedConfigBackend) Call(r Request) (Response, error) {
	c := r["config"].(Config)
	b.requests <- c
	<-b.release
	return Response{OK: true, Config: &c}, nil
}

func TestRapidConfigEditsStayOrderedAndDoNotRevertNewerUI(t *testing.T) {
	b := &delayedConfigBackend{requests: make(chan Config, 2), release: make(chan struct{}, 2)}
	s := newSettings(b, "0.0.1")
	changes := make(chan func(), 2)
	s.apply = func(fn func()) { changes <- fn }
	s.config.FontSize = 20
	s.saveConfig()
	first := <-b.requests
	s.config.FontSize = 24
	s.saveConfig()
	select {
	case <-b.requests:
		b.release <- struct{}{}
		b.release <- struct{}{}
		t.Fatal("new config was sent before the previous save completed")
	case <-time.After(50 * time.Millisecond):
	}
	b.release <- struct{}{}
	select {
	case change := <-changes:
		change()
	case <-time.After(time.Second):
		t.Fatal("first save did not complete")
	}
	if s.config.FontSize != 24 {
		t.Fatal("old response reverted the newer edit")
	}
	second := <-b.requests
	b.release <- struct{}{}
	(<-changes)()
	if first.FontSize != 20 || second.FontSize != 24 || s.config.FontSize != 24 {
		t.Fatalf("save order or final state changed: %d %d %d", first.FontSize, second.FontSize, s.config.FontSize)
	}
}

func TestCommandReplyDoesNotOverwriteANewerSettingsEdit(t *testing.T) {
	s, _ := newTestSettings()
	s.inline = false
	changes := make(chan func(), 1)
	s.apply = func(fn func()) { changes <- fn }
	s.run(op("toggle_mode"))
	oldReply := <-changes
	// A command read the old config, but its UI callback was queued behind a new edit.
	s.inline = true
	s.config.FontSize = 24
	s.saveConfig()
	oldReply()
	if s.config.FontSize != 24 {
		t.Fatal("an earlier command response reverted the saved font size")
	}
}

func TestOlderRefreshCannotOverwriteTheLatestSnapshot(t *testing.T) {
	s, backend := newTestSettings()
	s.inline = false
	changes := make(chan func(), 1)
	s.apply = func(fn func()) { changes <- fn }
	s.refresh()
	oldReply := <-changes
	backend.config.PageSize = 9
	s.refresh()
	newReply := <-changes
	newReply()
	oldReply()
	if s.config.PageSize != 9 {
		t.Fatal("an earlier refresh replaced the current settings snapshot")
	}
}

func TestConfigReadFailureIsVisibleAndKeepsTheLastSettings(t *testing.T) {
	s, backend := newTestSettings()
	backend.failRequest = "get_config"
	s.notice = ""
	s.refresh()
	if s.notice == "" || s.config.PageSize != 6 {
		t.Fatal("a failed config read must retain settings and show an error")
	}
}

func TestPhraseEditsWaitForTheSavedListAndKeepTheNextInput(t *testing.T) {
	for _, common := range []bool{false, true} {
		t.Run(map[bool]string{false: "quick", true: "common"}[common], func(t *testing.T) {
			s, backend := newTestSettings()
			s.phrases, backend.phrases = nil, nil
			s.common, backend.common = nil, nil
			s.inline = false
			changes := make(chan func(), 2)
			s.apply = func(fn func()) { changes <- fn }
			add := func(text string) {
				if common {
					s.newCommon = text
					s.addCommonPhrase()
				} else {
					s.newCode, s.newText = "cs", text
					s.addPhrase()
				}
			}
			add("第一条")
			first := <-changes
			add("第二条")
			// The first save's UI callback has not run; a second full-list write would lose it.
			select {
			case second := <-changes:
				second()
				t.Fatal("a second list mutation ran before the saved list was displayed")
			case <-time.After(30 * time.Millisecond):
			}
			first()
			if (common && s.newCommon != "第二条") || (!common && s.newText != "第二条") {
				t.Fatal("the first completion cleared the next input")
			}
			add("第二条")
			(<-changes)()
			if (common && len(backend.common) != 2) || (!common && len(backend.phrases) != 2) {
				t.Fatal("a saved phrase was lost")
			}
		})
	}
}
