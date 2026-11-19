package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/jpeg"
	"image/png"
	"os"
	"path/filepath"
	"testing"

	"github.com/egoist/mygo/ui"
)

func testBrandPNG(t *testing.T, fill color.RGBA) []byte {
	t.Helper()
	img := image.NewRGBA(image.Rect(0, 0, 32, 32))
	for y := 0; y < 32; y++ {
		for x := 0; x < 32; x++ {
			img.SetRGBA(x, y, fill)
		}
	}
	var encoded bytes.Buffer
	if err := png.Encode(&encoded, img); err != nil {
		t.Fatal(err)
	}
	return encoded.Bytes()
}

func testBrandResources(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	for _, entry := range []struct {
		path string
		fill color.RGBA
	}{
		{"logo.png", color.RGBA{255, 179, 250, 255}},
		{"white/logo.png", color.RGBA{255, 255, 255, 255}},
	} {
		path := filepath.Join(root, "icons", entry.path)
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, testBrandPNG(t, entry.fill), 0600); err != nil {
			t.Fatal(err)
		}
	}
	return root
}

func TestBrandLogoRepaintsWithoutRecreatingTheView(t *testing.T) {
	s, _ := newTestSettings()
	s.resources = testBrandResources(t)
	view := ui.NewTester(func(c *ui.Context) { s.logo(c, 32) }, 32, 32)
	for _, example := range []struct {
		variant string
		custom  string
		want    color.RGBA
	}{
		{"default", "", color.RGBA{255, 179, 250, 255}},
		{"white", "", color.RGBA{255, 255, 255, 255}},
		{"custom", base64.StdEncoding.EncodeToString(testBrandPNG(t, color.RGBA{0, 100, 200, 255})), color.RGBA{0, 100, 200, 255}},
		{"custom", base64.StdEncoding.EncodeToString(testBrandPNG(t, color.RGBA{0, 160, 80, 255})), color.RGBA{0, 160, 80, 255}},
		{"default", "", color.RGBA{255, 179, 250, 255}},
	} {
		payload, _ := json.Marshal(map[string]string{"brand_icon": example.variant, "custom_brand_icon": example.custom})
		if err := json.Unmarshal(payload, &s.config); err != nil {
			t.Fatal(err)
		}
		view.Frame()
		if got := view.Image().RGBAAt(16, 16); got != example.want {
			t.Fatalf("切换到 %s 后仍显示旧图标：%v，需要 %v", example.variant, got, example.want)
		}
	}
}

func TestBrandAppearancePageOffersLiveSelection(t *testing.T) {
	s, b := newTestSettings()
	view := open(t, s, "候选窗与外观")
	if !view.HasText("品牌图标") || !view.HasText("自定义…") {
		t.Fatalf("缺少品牌图标选择：%q", view.Texts())
	}
	brand, _ := view.Find("品牌图标")
	lastCandidateSetting, _ := view.Find("补充字体")
	if brand.Y <= lastCandidateSetting.Y+lastCandidateSetting.H {
		t.Fatal("品牌外观必须放在所有候选窗设置之后")
	}
	if err := view.Click("默认"); err != nil {
		t.Fatal(err)
	}
	if err := view.Click("白色"); err != nil {
		t.Fatal(err)
	}
	view.Frame()
	value, _ := json.Marshal(b.config)
	var saved map[string]any
	_ = json.Unmarshal(value, &saved)
	if saved["brand_icon"] != "white" {
		t.Fatalf("品牌图标选择没有立即保存：%s", value)
	}
}

func TestBrandCustomImportSurvivesSourceChangesAndSettingsReload(t *testing.T) {
	s, backend := newTestSettings()
	path := filepath.Join(t.TempDir(), "my-logo.png")
	original := testBrandPNG(t, color.RGBA{0, 100, 200, 255})
	if err := os.WriteFile(path, original, 0600); err != nil {
		t.Fatal(err)
	}
	s.openIcon = func() (string, error) { return path, nil }
	var applied []string
	s.onBrandIcon = func(config Config) error {
		if backend.config.BrandIcon != config.BrandIcon {
			t.Fatal("图标在设置保存成功前被更新")
		}
		applied = append(applied, config.BrandIcon)
		return nil
	}
	view := open(t, s, "候选窗与外观")
	if err := view.Click("自定义…"); err != nil {
		t.Fatal(err)
	}
	view.Frame()
	if backend.config.BrandIcon != "custom" || backend.config.CustomBrandIcon == "" || len(applied) != 1 || applied[0] != "custom" {
		t.Fatalf("自定义图标没有保存并立即应用：%q / %v", backend.config.BrandIcon, applied)
	}
	// 原文件改变后，显示和重新打开设置仍读取保存的副本。
	if err := os.WriteFile(path, testBrandPNG(t, color.RGBA{0, 160, 80, 255}), 0600); err != nil {
		t.Fatal(err)
	}
	reloaded := newSettings(backend, "0.0.1")
	reloaded.inline = true
	reloaded.refresh()
	for _, entry := range []struct {
		name string
		size int
	}{{"tray.png", 32}, {"logo.png", 128}} {
		data, err := loadBrandIcon("", reloaded.config, entry.name)
		if err != nil {
			t.Fatal(err)
		}
		img, err := png.Decode(bytes.NewReader(data))
		if err != nil || img.Bounds().Dx() != entry.size || img.Bounds().Dy() != entry.size {
			t.Fatalf("%s 尺寸错误：%v", entry.name, err)
		}
		if got := color.RGBAModel.Convert(img.At(entry.size/2, entry.size/2)); got != (color.RGBA{0, 100, 200, 255}) {
			t.Fatalf("图标依赖了原文件的新内容：%v", got)
		}
	}
	current := "自定义"
	for _, choice := range []string{"白色", "自定义", "默认", "自定义"} {
		if err := view.Click(current); err != nil {
			t.Fatal(err)
		}
		if err := view.Click(choice); err != nil {
			t.Fatal(err)
		}
		view.Frame()
		current = choice
	}
	if backend.config.CustomBrandIcon != reloaded.config.CustomBrandIcon || backend.config.BrandIcon != "custom" {
		t.Fatal("切换内置外观后丢失了导入图标")
	}
}

func TestBrandImportCancelErrorsAndSaveFailureKeepPreviousIcon(t *testing.T) {
	for _, problem := range []string{"cancel", "dialog", "image", "save"} {
		t.Run(problem, func(t *testing.T) {
			s, backend := newTestSettings()
			path := filepath.Join(t.TempDir(), "logo.png")
			data := testBrandPNG(t, color.RGBA{0, 100, 200, 255})
			if problem == "image" {
				data = []byte("not an image")
			}
			if err := os.WriteFile(path, data, 0600); err != nil {
				t.Fatal(err)
			}
			s.openIcon = func() (string, error) {
				switch problem {
				case "cancel":
					return "", nil
				case "dialog":
					return "", fmt.Errorf("文件选择失败")
				default:
					return path, nil
				}
			}
			if problem == "save" {
				backend.failRequest = "set_config"
			}
			applied := "default"
			s.onBrandIcon = func(config Config) error { applied = config.BrandIcon; return nil }
			view := open(t, s, "候选窗与外观")
			if err := view.Click("自定义…"); err != nil {
				t.Fatal(err)
			}
			view.Frame()
			if applied != "default" || backend.config.BrandIcon != "default" || s.config.BrandIcon != "default" || s.brandIconBusy {
				t.Fatalf("失败或取消时改变了图标：%q / %q，已应用 %v", backend.config.BrandIcon, s.config.BrandIcon, applied)
			}
			if problem == "cancel" && backend.last("set_config") != nil {
				t.Fatal("取消选择仍保存了设置")
			}
		})
	}
}

func TestBrandImportPreservesAspectRatioAndGeneratesRuntimeSizes(t *testing.T) {
	for _, format := range []string{"png", "jpeg"} {
		t.Run(format, func(t *testing.T) {
			img := image.NewRGBA(image.Rect(0, 0, 64, 32))
			for y := 0; y < 32; y++ {
				for x := 0; x < 64; x++ {
					img.SetRGBA(x, y, color.RGBA{0, 100, 200, 255})
				}
			}
			var encoded bytes.Buffer
			var err error
			if format == "jpeg" {
				err = jpeg.Encode(&encoded, img, nil)
			} else {
				err = png.Encode(&encoded, img)
			}
			if err != nil {
				t.Fatal(err)
			}
			path := filepath.Join(t.TempDir(), "wide-logo."+format)
			if err := os.WriteFile(path, encoded.Bytes(), 0600); err != nil {
				t.Fatal(err)
			}
			custom, err := importBrandIcon(path)
			if err != nil {
				t.Fatal(err)
			}
			data, _ := base64.StdEncoding.DecodeString(custom)
			normalized, err := png.Decode(bytes.NewReader(data))
			if err != nil || normalized.Bounds() != image.Rect(0, 0, 256, 256) {
				t.Fatalf("图像没有规范为 256 px PNG：%v", err)
			}
			for _, point := range []image.Point{{128, 0}, {128, 63}, {128, 192}, {128, 255}} {
				if _, _, _, alpha := normalized.At(point.X, point.Y).RGBA(); alpha != 0 {
					t.Fatalf("横图比例被改变或透明留白丢失：%v", point)
				}
			}
			if _, _, _, alpha := normalized.At(128, 128).RGBA(); alpha != 65535 {
				t.Fatal("图像中央内容丢失")
			}
		})
	}
}

func TestBrandAppearanceProductionViewAtWindowSizes(t *testing.T) {
	resources := t.TempDir()
	for _, entry := range []struct{ source, path string }{
		{"freetype-logo.png", "logo.png"},
		{"freetype-white-logo.png", "white/logo.png"},
	} {
		data, err := os.ReadFile(filepath.Join("..", "assets", entry.source))
		if err != nil {
			t.Fatal(err)
		}
		img, err := png.Decode(bytes.NewReader(data))
		if err != nil {
			t.Fatal(err)
		}
		data, err = brandImagePNG(img, 128)
		if err != nil {
			t.Fatal(err)
		}
		path := filepath.Join(resources, "icons", entry.path)
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	for _, variant := range []string{"default", "white"} {
		for _, dark := range []bool{false, true} {
			for _, size := range [][2]int{{1000, 720}, {780, 540}} {
				s, _ := newTestSettings()
				s.resources, s.page, s.config.BrandIcon = resources, "candidates", variant
				view := ui.NewTester(s.view, size[0], size[1])
				view.SetDark(dark)
				view.Scroll(float32(size[0]-50), float32(size[1]-50), 0, 10000)
				view.Frame()
				label := "默认"
				if variant == "white" {
					label = "白色"
				}
				for _, text := range []string{label, "自定义…"} {
					bounds, found := view.Find(text)
					if !found || bounds.X < 236 || bounds.X+bounds.W > float32(size[0]) || bounds.Y+bounds.H > float32(size[1]) {
						t.Fatalf("%s 深色=%v %v 的图标控件不完整：%s %+v", variant, dark, size, text, bounds)
					}
				}
				saveUIFrame(t, fmt.Sprintf("brand-%s-dark-%v-%d", variant, dark, size[0]), view)
			}
		}
	}
}
