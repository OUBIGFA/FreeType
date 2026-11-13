package main

import (
	"slices"
	"strings"
	"testing"
)

func TestAboutLinksOpenTheActualProjectAndFeedback(t *testing.T) {
	s, _ := newTestSettings()
	var opened []string
	s.openURL = func(url string) error { opened = append(opened, url); return nil }
	tt := open(t, s, "关于")
	for _, text := range tt.Texts() {
		if strings.Contains(text, "WowType") || strings.Contains(text, "mygo") || strings.Contains(text, "测试版") {
			t.Fatalf("产品介绍不应包含内部实现或阶段文字：%q", text)
		}
	}
	for _, label := range []string{"打开仓库", "反馈问题", "查看开源说明"} {
		if err := tt.Click(label); err != nil {
			t.Fatal(err)
		}
	}
	want := []string{
		"https://github.com/OUBIGFA/FreeType",
		"https://github.com/OUBIGFA/FreeType/issues",
		"https://github.com/OUBIGFA/FreeType/tree/main/third_party",
	}
	if !slices.Equal(opened, want) {
		t.Fatalf("链接应指向实际项目：%v", opened)
	}
}
