package main

import (
	"bytes"
	"encoding/base64"
	"encoding/binary"
	"fmt"
	"image"
	_ "image/jpeg"
	"image/png"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"

	"github.com/egoist/mygo"

	"golang.org/x/image/draw"
	_ "golang.org/x/image/webp"
)

// 自定义图标在导入时变成有透明留白的方形 PNG。配置携带图像内容，原文件可移动，
// 恢复设置和同步也能继续显示；不把本机绝对路径写进配置。
func importBrandIcon(path string) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer file.Close()
	const maxFileSize = 10 << 20
	data, err := io.ReadAll(io.LimitReader(file, maxFileSize+1))
	if err != nil {
		return "", err
	}
	if len(data) > maxFileSize {
		return "", fmt.Errorf("图像文件不能超过 10 MB")
	}
	info, format, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil || (format != "png" && format != "jpeg" && format != "webp") {
		return "", fmt.Errorf("请选择有效的 PNG、JPEG 或 WebP 图像")
	}
	if info.Width < 1 || info.Height < 1 || info.Width > 4096 || info.Height > 4096 {
		return "", fmt.Errorf("图像宽高必须在 1–4096 像素之间")
	}
	img, _, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return "", fmt.Errorf("读取图像失败：%w", err)
	}
	encoded, err := brandImagePNG(img, 256)
	if err != nil {
		return "", err
	}
	return base64.StdEncoding.EncodeToString(encoded), nil
}

// 从原图缩放并居中，保留图像比例及透明度，不裁掉自定义图标的边缘。
func brandImagePNG(img image.Image, size int) ([]byte, error) {
	bounds := img.Bounds()
	width, height := size, size
	if bounds.Dx() > bounds.Dy() {
		height = max(1, size*bounds.Dy()/bounds.Dx())
	} else {
		width = max(1, size*bounds.Dx()/bounds.Dy())
	}
	resized := image.NewNRGBA(image.Rect(0, 0, size, size))
	x, y := (size-width)/2, (size-height)/2
	draw.CatmullRom.Scale(resized, image.Rect(x, y, x+width, y+height), img, bounds, draw.Over, nil)
	var encoded bytes.Buffer
	if err := png.Encode(&encoded, resized); err != nil {
		return nil, err
	}
	return encoded.Bytes(), nil
}

// ICO 里的每一幅 PNG 都从导入原图缩放，避免系统在高 DPI 下放大低分辨率预览。
func brandImageICO(img image.Image) ([]byte, error) {
	sizes := []int{16, 20, 24, 32, 40, 48, 64, 96, 128, 256}
	data := make([]byte, 6+16*len(sizes))
	binary.LittleEndian.PutUint16(data[2:4], 1)
	binary.LittleEndian.PutUint16(data[4:6], uint16(len(sizes)))
	for index, size := range sizes {
		png, err := brandImagePNG(img, size)
		if err != nil {
			return nil, err
		}
		entry := data[6+16*index : 6+16*(index+1)]
		entry[0], entry[1] = byte(size), byte(size) // ICO 用 0 表示 256 px。
		binary.LittleEndian.PutUint16(entry[4:6], 1)
		binary.LittleEndian.PutUint16(entry[6:8], 32)
		binary.LittleEndian.PutUint32(entry[8:12], uint32(len(png)))
		binary.LittleEndian.PutUint32(entry[12:16], uint32(len(data)))
		data = append(data, png...)
	}
	return data, nil
}

// 内置图标使用构建时从完整源图生成的各尺寸；自定义图标按相同运行时尺寸输出。
func loadBrandIcon(resources string, config Config, name string) ([]byte, error) {
	if config.BrandIcon != "custom" {
		dir := filepath.Join(resources, "icons")
		if config.BrandIcon == "white" {
			dir = filepath.Join(dir, "white")
		}
		return os.ReadFile(filepath.Join(dir, name))
	}
	data, err := base64.StdEncoding.DecodeString(config.CustomBrandIcon)
	if err != nil {
		return nil, fmt.Errorf("自定义图标数据损坏：%w", err)
	}
	info, err := png.DecodeConfig(bytes.NewReader(data))
	if err != nil || info.Width > 256 || info.Height > 256 {
		return nil, fmt.Errorf("自定义图标数据损坏，请重新导入图像")
	}
	img, err := png.Decode(bytes.NewReader(data))
	if err != nil {
		return nil, fmt.Errorf("自定义图标不是有效 PNG：%w", err)
	}
	if name == "freetype.ico" {
		return brandImageICO(img)
	}
	size := 128
	if name == "tray.png" {
		size = 32
	}
	return brandImagePNG(img, size)
}

// 只传选中的图像。提权助手无需读取用户的整份配置，也不会以管理员身份常驻。
func updateSystemBrandIcon(resources string, config Config) error {
	icon, err := loadBrandIcon(resources, config, "freetype.ico")
	if err != nil {
		return err
	}
	root := filepath.Join(os.TempDir(), "_temp")
	if err := os.MkdirAll(root, 0700); err != nil {
		return err
	}
	directory, err := os.MkdirTemp(root, "freetype-brand-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(directory)
	path := filepath.Join(directory, "brand.ico")
	if err := os.WriteFile(path, icon, 0600); err != nil {
		return err
	}
	cmd := exec.Command(serverExe(resources), "--set-brand-icon", path)
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: 0x08000000}
	output, err := cmd.CombinedOutput()
	if err != nil {
		if detail := strings.TrimSpace(string(output)); detail != "" {
			return fmt.Errorf("%s", detail)
		}
		return fmt.Errorf("系统图标更新未完成：%w", err)
	}
	return nil
}

// 窗口和托盘在主线程立即换图；系统资源按保存顺序在后台更新，UAC 等待不阻塞界面。
func (a *app) applyBrandIcon(config Config) error {
	if err := a.setBrandIcon(config); err != nil {
		return err
	}
	selected := Config{BrandIcon: config.BrandIcon}
	if config.BrandIcon == "custom" {
		selected.CustomBrandIcon = config.CustomBrandIcon
	}
	if a.systemBrandTail != nil && selected.BrandIcon == a.systemBrandConfig.BrandIcon &&
		selected.CustomBrandIcon == a.systemBrandConfig.CustomBrandIcon && a.systemBrandError == "" {
		return nil
	}
	a.systemBrandConfig = selected
	previousError := a.systemBrandError
	a.systemBrandError = ""
	a.systemBrandRevision++
	revision := a.systemBrandRevision
	previous, done := a.systemBrandTail, make(chan struct{})
	a.systemBrandTail = done
	go func() {
		if previous != nil {
			<-previous
		}
		err := updateSystemBrandIcon(a.resources, selected)
		close(done)
		mygo.RunOnMain(func() {
			if revision != a.systemBrandRevision {
				return
			}
			if err != nil {
				a.systemBrandError = "设置已保存，系统输入指示图标更新失败：" + err.Error()
			}
			if a.settings != nil && a.win != nil {
				s := a.settings
				a.win.Update(func() {
					if err != nil || previousError != "" && s.notice == previousError {
						s.notice = a.systemBrandError
					}
				})
			}
		})
	}()
	return nil
}

func (s *settings) chooseBrandIcon() {
	s.brandIconBusy = true
	s.background(func() func() {
		path, err := s.openIcon()
		if err != nil {
			return func() {
				s.brandIconBusy = false
				s.notice = "选择图标失败：" + err.Error()
			}
		}
		if path == "" {
			return func() { s.brandIconBusy = false }
		}
		data, err := importBrandIcon(path)
		return func() {
			s.brandIconBusy = false
			if err != nil {
				s.notice = "导入图标失败：" + err.Error()
				return
			}
			s.config.BrandIcon, s.config.CustomBrandIcon = "custom", data
			s.saveConfig()
		}
	})
}

// 保存、重新读取和恢复设置共用同一回调，关闭设置窗口也不丢失托盘外观。
func (s *settings) updateBrandIcon() {
	s.savedBrandIcon, s.savedCustomBrandIcon = s.config.BrandIcon, s.config.CustomBrandIcon
	if s.onBrandIcon != nil {
		if err := s.onBrandIcon(s.config); err != nil {
			s.notice = "更新图标失败：" + err.Error()
		}
	}
}
