package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/egoist/mygo/netproxy"
)

const novaModelURL = "https://github.com/nova-ime/nova-model/releases/download/v1/gpt2.gguf"
const novaModelSHA = "b72e0c8728e05454677373c700aba7860c8595c589d73b39683e3de2671708e3"
const novaModelBytes int64 = 116198144

type EnhancementStatus struct {
	Enabled bool   `json:"enabled"`
	State   string `json:"state"`
	Error   string `json:"error"`
}

// Download to an exclusive sibling. A complete length and digest are required
// before the model becomes visible to the service. Downloading never enables it.
func downloadEnhancement(ctx context.Context, client *http.Client, url, target, digest string, size int64) error {
	if err := os.MkdirAll(filepath.Dir(target), 0700); err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return err
	}
	response, err := client.Do(req)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return fmt.Errorf("下载失败：%s", response.Status)
	}
	file, err := os.CreateTemp(filepath.Dir(target), "model-*.partial")
	if err != nil {
		return err
	}
	temp := file.Name()
	defer func() { file.Close(); os.Remove(temp) }()
	sum := sha256.New()
	n, err := io.Copy(io.MultiWriter(file, sum), io.LimitReader(response.Body, size+1))
	if err != nil {
		return err
	}
	if n != size {
		return fmt.Errorf("模型大小不正确：收到 %d 字节", n)
	}
	if hex.EncodeToString(sum.Sum(nil)) != digest {
		return fmt.Errorf("模型校验失败，请重新下载")
	}
	if err = file.Sync(); err != nil {
		return err
	}
	if err = file.Close(); err != nil {
		return err
	}
	// A verified existing file needs no replacement and may be in use.
	if _, err = os.Stat(target); err == nil {
		return fmt.Errorf("模型已存在，请先删除后重新下载")
	}
	return os.Rename(temp, target)
}

func (s *settings) modelPath() string {
	return filepath.Join(s.status.DataDir, "enhancement", "nova-v1.gguf")
}
func (s *settings) refreshEnhancement() {
	if s.enhancementRefreshing || time.Since(s.enhancementChecked) < time.Second {
		return
	}
	s.enhancementRefreshing = true
	s.enhancementChecked = time.Now()
	generation := s.enhancementGeneration
	path := s.modelPath()
	s.background(func() func() {
		response, err := s.backend.Call(op("enhancement_status"))
		deadline := time.Now().Add(15 * time.Second)
		for err == nil && response.Enhancement != nil && (response.Enhancement.State == "loading" || response.Enhancement.State == "stopping") && time.Now().Before(deadline) {
			time.Sleep(100 * time.Millisecond)
			response, err = s.backend.Call(op("enhancement_status"))
		}
		_, fileErr := os.Stat(path)
		return func() {
			s.enhancementRefreshing = false
			if generation != s.enhancementGeneration {
				return
			}
			s.enhancementDownloaded = fileErr == nil
			if err == nil && response.Enhancement != nil {
				s.enhancement = *response.Enhancement
			}
		}
	})
}
func (s *settings) downloadEnhancement() {
	if s.enhancementBusy {
		return
	}
	s.enhancementBusy = true
	s.enhancementGeneration++
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Minute)
	s.enhancementCancel = cancel
	target := s.modelPath()
	s.background(func() func() {
		defer cancel()
		transport := netproxy.Transport().Clone()
		transport.ResponseHeaderTimeout = 30 * time.Second
		err := downloadEnhancement(ctx, &http.Client{Transport: transport, CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if req.URL.Scheme != "https" {
				return fmt.Errorf("模型下载不接受非 HTTPS 跳转")
			}
			if len(via) >= 10 {
				return fmt.Errorf("模型下载跳转次数过多")
			}
			return nil
		}}, novaModelURL, target, novaModelSHA, novaModelBytes)
		return func() {
			s.enhancementBusy = false
			s.enhancementCancel = nil
			if err != nil {
				s.notice = err.Error()
			} else {
				s.enhancementDownloaded = true
				s.notice = "模型已下载；启用后使用"
			}
		}
	})
}
func (s *settings) setEnhancement(enabled bool) {
	if s.enhancementBusy {
		return
	}
	s.enhancementBusy = true
	s.enhancementGeneration++
	s.background(func() func() {
		response, err := s.backend.Call(op("set_enhancement", "enabled", enabled))
		deadline := time.Now().Add(15 * time.Second)
		for err == nil && response.Enhancement != nil && (response.Enhancement.State == "loading" || response.Enhancement.State == "stopping") && time.Now().Before(deadline) {
			time.Sleep(100 * time.Millisecond)
			response, err = s.backend.Call(op("enhancement_status"))
		}
		return func() {
			s.enhancementBusy = false
			if err != nil {
				s.notice = err.Error()
			} else if response.Enhancement != nil {
				s.enhancement = *response.Enhancement
			}
		}
	})
}
func (s *settings) removeEnhancement() {
	if s.enhancementBusy {
		return
	}
	s.enhancementBusy = true
	s.enhancementGeneration++
	target := s.modelPath()
	s.background(func() func() {
		_, err := s.backend.Call(op("set_enhancement", "enabled", false))
		if err == nil {
			deadline := time.Now().Add(10 * time.Second)
			for {
				r, e := s.backend.Call(op("enhancement_status"))
				if e != nil {
					err = e
					break
				}
				if r.Enhancement != nil && r.Enhancement.State == "off" {
					err = trashFile(target)
					break
				}
				if time.Now().After(deadline) {
					err = fmt.Errorf("模型仍在释放，请稍后重试")
					break
				}
				time.Sleep(100 * time.Millisecond)
			}
		}
		return func() {
			s.enhancementBusy = false
			if err != nil {
				s.notice = err.Error()
			} else {
				s.enhancementDownloaded = false
				s.enhancement = EnhancementStatus{State: "off"}
				s.notice = "增强模型已移到回收站"
			}
		}
	})
}
