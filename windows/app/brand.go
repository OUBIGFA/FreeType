package main

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"image"
	_ "image/jpeg"
	"image/png"
	"io"
	"os"
	"path/filepath"

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
	size := 128
	if name == "tray.png" {
		size = 32
	}
	return brandImagePNG(img, size)
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
