package main

import (
	"bufio"
	"errors"
	"fmt"
	"os"
	"testing"
	"time"

	"golang.org/x/sys/windows"
)

type startingBackend struct {
	states []Status
}

func (b *startingBackend) Call(Request) (Response, error) {
	if len(b.states) == 0 {
		return Response{}, errors.New("unexpected extra startup poll")
	}
	state := b.states[0]
	b.states = b.states[1:]
	return Response{OK: true, Status: &state}, nil
}

func TestEnsureServerWaitsForDictionaryAndReportsStartupFailure(t *testing.T) {
	for _, sample := range []struct {
		name string
		last Status
		fail bool
	}{
		{"ready", Status{Ready: true}, false},
		{"failed", Status{Error: "词库文件已损坏"}, true},
	} {
		t.Run(sample.name, func(t *testing.T) {
			backend := &startingBackend{states: []Status{{Ready: false}, sample.last}}
			err := ensureServer(backend, t.TempDir(), time.Second)
			if (err != nil) != sample.fail || len(backend.states) != 0 {
				t.Fatalf("startup returned before its final state: remaining=%d err=%v", len(backend.states), err)
			}
		})
	}
}

func TestPipeTimeoutReleasesQueueWithoutReplayingMutation(t *testing.T) {
	path := fmt.Sprintf(`\\.\pipe\FreeType-audit-%d-%d`, os.Getpid(), time.Now().UnixNano())
	name, _ := windows.UTF16PtrFromString(path)
	handle, err := windows.CreateNamedPipe(name, windows.PIPE_ACCESS_DUPLEX, windows.PIPE_TYPE_BYTE|windows.PIPE_WAIT, 1, 4096, 4096, 0, nil)
	if err != nil {
		t.Fatal(err)
	}
	server := os.NewFile(uintptr(handle), path)
	defer server.Close()
	received := make(chan struct{})
	release := make(chan struct{})
	finished := make(chan struct{})
	go func() {
		defer close(finished)
		if err := windows.ConnectNamedPipe(handle, nil); err != nil && err != windows.ERROR_PIPE_CONNECTED {
			return
		}
		_, _ = bufio.NewReader(server).ReadBytes('\n')
		close(received)
		<-release
		_, _ = server.Write([]byte("{\"ok\":true}\n"))
		_ = windows.DisconnectNamedPipe(handle)
	}()
	client := &pipeClient{path: path, timeout: 50 * time.Millisecond}
	defer client.close()
	result := make(chan error, 1)
	go func() { _, err := client.Call(op("toggle_mode")); result <- err }()
	<-received
	select {
	case err := <-result:
		close(release)
		<-finished
		if err == nil {
			t.Fatal("a silent server must return an error")
		}
		if client.file != nil {
			t.Fatal("timed-out connection must be discarded")
		}
	case <-time.After(time.Second):
		close(release)
		<-finished
		<-result
		t.Fatal("a silent server held the settings request queue indefinitely")
	}
	// The next call uses a fresh pipe after the failed request released the mutex.
	server.Close()
	nextHandle, err := windows.CreateNamedPipe(name, windows.PIPE_ACCESS_DUPLEX, windows.PIPE_TYPE_BYTE|windows.PIPE_WAIT, 1, 4096, 4096, 0, nil)
	if err != nil {
		t.Fatal(err)
	}
	next := os.NewFile(uintptr(nextHandle), path)
	defer next.Close()
	go func() {
		_ = windows.ConnectNamedPipe(nextHandle, nil)
		_, _ = bufio.NewReader(next).ReadBytes('\n')
		_, _ = next.Write([]byte("{\"ok\":true}\n"))
	}()
	response, err := client.Call(op("status"))
	if err != nil || !response.OK {
		t.Fatalf("request queue did not recover after timeout: %v", err)
	}
}
