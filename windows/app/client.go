package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"syscall"
	"time"

	"golang.org/x/sys/windows"
)

// pipeClient 连 \\.\pipe\FreeType-<SID>；断了下次调用时重连。
type pipeClient struct {
	mu      sync.Mutex
	path    string
	file    *os.File
	reader  *bufio.Reader
	timeout time.Duration // 零值使用管理请求的默认上限；测试可缩短。
}

func currentUserSID() (string, error) {
	user, err := windows.GetCurrentProcessToken().GetTokenUser()
	if err != nil {
		return "", err
	}
	return user.User.Sid.String(), nil
}

func newPipeClient() (*pipeClient, error) {
	sid, err := currentUserSID()
	if err != nil {
		return nil, fmt.Errorf("无法读取当前用户：%w", err)
	}
	suffix := ""
	if inst := os.Getenv("FREETYPE_INSTANCE"); inst != "" {
		suffix = "-" + inst
	}
	return &pipeClient{path: `\\.\pipe\FreeType-` + sid + suffix}, nil
}

func (p *pipeClient) close() {
	if p.file != nil {
		p.file.Close()
	}
	p.file, p.reader = nil, nil
}

// Call 发一条请求。服务进程不在时返回错误。
func (p *pipeClient) Call(req Request) (Response, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for attempt := 0; attempt < 2; attempt++ {
		if p.file == nil {
			name, err := windows.UTF16PtrFromString(p.path)
			if err != nil {
				return Response{}, err
			}
			// 重叠句柄交给 Go 的轮询器，SetDeadline 才能真正取消管道等待。
			handle, err := windows.CreateFile(name, windows.GENERIC_READ|windows.GENERIC_WRITE,
				0, nil, windows.OPEN_EXISTING, windows.FILE_FLAG_OVERLAPPED, 0)
			if err != nil {
				return Response{}, fmt.Errorf("无法连接输入法服务：%w", err)
			}
			f := os.NewFile(uintptr(handle), p.path)
			p.file, p.reader = f, bufio.NewReader(f)
		}
		timeout := p.timeout
		if timeout <= 0 {
			timeout = 30 * time.Second
		}
		if err := p.file.SetDeadline(time.Now().Add(timeout)); err != nil {
			p.close()
			return Response{}, fmt.Errorf("无法设置服务应答期限：%w", err)
		}
		line, err := json.Marshal(req)
		if err != nil {
			return Response{}, err
		}
		if _, err = p.file.Write(append(line, '\n')); err == nil {
			var reply []byte
			if reply, err = p.reader.ReadBytes('\n'); err == nil {
				var resp Response
				if err := json.Unmarshal(reply, &resp); err != nil {
					p.close()
					return Response{}, err
				}
				if !resp.OK {
					return resp, errors.New(resp.Error)
				}
				return resp, nil
			}
			// 请求已发出时结果可能已生效，断连或超时都不能重放切换/导入等操作。
			p.close()
			return Response{}, fmt.Errorf("输入法服务未返回结果，请刷新后检查操作是否生效：%w", err)
		}
		// 连接断了（服务进程重启过）：重连再试一次
		p.close()
		if errors.Is(err, os.ErrDeadlineExceeded) {
			return Response{}, fmt.Errorf("输入法服务应答超时：%w", err)
		}
	}
	return Response{}, errors.New("输入法服务没有应答")
}

// serverExe 是随安装包的服务进程：<安装目录>\bin\FreeTypeServer.exe。
func serverExe(resources string) string {
	return filepath.Join(resources, "bin", "FreeTypeServer.exe")
}

// ensureServer 在服务进程没跑时启动它，最多等 timeout。
func ensureServer(b Backend, resources string, timeout time.Duration) error {
	if _, err := b.Call(op("status")); err == nil {
		return nil
	}
	exe := serverExe(resources)
	if _, err := os.Stat(exe); err != nil {
		return fmt.Errorf("找不到输入法服务程序 %s", exe)
	}
	cmd := exec.Command(exe)
	const detachedProcess = 0x00000008
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: detachedProcess}
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("启动输入法服务失败：%w", err)
	}
	_ = cmd.Process.Release()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if _, err := b.Call(op("status")); err == nil {
			return nil
		}
		time.Sleep(100 * time.Millisecond)
	}
	return errors.New("输入法服务启动后没有应答")
}
