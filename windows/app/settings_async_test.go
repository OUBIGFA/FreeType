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
