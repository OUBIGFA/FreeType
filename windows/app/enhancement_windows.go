package main

import (
	"fmt"
	"github.com/egoist/mygo/ui"
	"golang.org/x/sys/windows"
	"syscall"
	"unsafe"
)

func (s *settings) enhancementRow(c *ui.Context) {
	s.refreshEnhancement()
	label := "未下载；基础词块编辑照常可用"
	if s.enhancementDownloaded {
		label = "已下载；未启用"
	}
	if s.enhancement.Enabled {
		label = map[string]string{"loading": "正在加载", "ready": "已启用", "error": "加载失败"}[s.enhancement.State]
	}
	if s.enhancementBusy {
		label = "正在处理"
	}
	s.settingRowValue(c, "features/Nova 增强模型", label, func() {
		if !s.enhancementDownloaded && ui.Button(c, "下载约 116 MB").Disabled(s.enhancementBusy).Clicked() {
			s.downloadEnhancement()
		}
		if s.enhancementDownloaded && !s.enhancement.Enabled && ui.Button(c, "启用").Disabled(s.enhancementBusy).Clicked() {
			s.setEnhancement(true)
		}
		if s.enhancement.Enabled && ui.Button(c, "关闭").Disabled(s.enhancementBusy).Clicked() {
			s.setEnhancement(false)
		}
		if s.enhancementDownloaded && ui.Button(c, "删除").Disabled(s.enhancementBusy).Clicked() {
			s.confirmEnhancementDelete = true
		}
		if s.enhancementCancel != nil && ui.Button(c, "取消下载").Clicked() {
			s.enhancementCancel()
		}
		if ui.Button(c, "模型说明").Clicked() {
			s.openLink("https://github.com/nova-ime/nova-model")
		}
	})
	if s.enhancement.Error != "" {
		s.settingRowValue(c, "features/Nova 增强模型", s.enhancement.Error, func() {})
	}
	if ui.AlertDialog(c, &s.confirmEnhancementDelete, "删除增强模型", "关闭增强模型并将下载文件移到回收站；基础输入和学习记录保留", "取消", "删除") == 1 {
		s.removeEnhancement()
	}
}

func trashFile(path string) error {
	// A double-NUL list is required by SHFileOperation. Shell errors leave the
	// file in place; there is deliberately no permanent-delete fallback.
	chars, err := syscall.UTF16FromString(path)
	if err != nil {
		return err
	}
	chars = append(chars, 0)
	type operation struct {
		Window   uintptr
		Function uint32
		From, To *uint16
		Flags    uint16
		Aborted  int32
		Mappings uintptr
		Title    *uint16
	}
	op := operation{Function: 3, From: &chars[0], Flags: 0x40 | 0x10 | 0x400 | 0x4}
	code, _, _ := windows.NewLazySystemDLL("shell32.dll").NewProc("SHFileOperationW").Call(uintptr(unsafe.Pointer(&op)))
	if code != 0 || op.Aborted != 0 {
		return fmt.Errorf("无法将模型移到回收站，文件已保留（%d）", code)
	}
	return nil
}
