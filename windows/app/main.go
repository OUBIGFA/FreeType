// FreeType.exe：托盘图标和设置窗口。
//
// 开机时以 --background 启动，只放托盘图标并确保输入法服务（bin\FreeTypeServer.exe）在跑；
// 从开始菜单打开时显示设置窗口。再次启动会把设置窗口带到前面。
package main

import (
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"

	// 语言目录：安装器和 mygo 自己的控件文字按系统语言显示（简体中文、繁体中文、英文）
	_ "github.com/oubigfa/freetype/windows/app/locales"
)

type app struct {
	client    Backend
	resources string
	win       *mygo.Window
	settings  *settings
}

func (a *app) openSettings() {
	if a.win != nil {
		a.win.Show()
		a.win.Restore()
		a.win.Focus()
		a.settings.refresh()
		return
	}
	s := newSettings(a.client, mygo.App.Version())
	s.resources = a.resources
	s.openFolder = func(path string) { _ = exec.Command("explorer.exe", path).Start() }
	s.openTable = func(title string) string {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
			Parent:  a.win,
			Title:   title,
			Filters: []mygo.FileFilter{{Name: "词库文件", Extensions: []string{"tsv", "txt"}}},
		})
		if err != nil || len(paths) == 0 {
			return ""
		}
		return paths[0]
	}
	s.openFile = func(title string) string {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
			Parent:  a.win,
			Title:   title,
			Filters: []mygo.FileFilter{{Name: "词库、短语文件", Extensions: []string{"txt", "tsv", "scel", "csv"}}},
		})
		if err != nil || len(paths) == 0 {
			return ""
		}
		return paths[0]
	}
	s.saveFile = func(title, name string) string {
		path, err := mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: a.win, Title: title, DefaultPath: name})
		if err != nil {
			return ""
		}
		return path
	}
	win := mygo.NewWindow(mygo.WindowOptions{
		Title:     "FreeType 设置",
		Width:     1000,
		Height:    720,
		MinWidth:  780,
		MinHeight: 540,
		// 不画系统标题栏的图标和文字，只留右上角的窗口按钮
		TitleBarStyle: mygo.TitleBarHidden,
		StateKey:      "settings",
		Content:       ui.View(s.view),
	})
	s.apply = func(fn func()) { win.Update(fn) }
	win.OnClosed(func() { a.win, a.settings = nil, nil })
	a.win, a.settings = win, s
	s.refresh()
}

func (a *app) makeTray() {
	icon, err := os.ReadFile(filepath.Join(a.resources, "icons", "tray.png"))
	if err != nil {
		log.Println("tray icon:", err)
	}
	menu := mygo.NewMenu([]*mygo.MenuItem{
		{Label: "打开设置", Click: func(*mygo.MenuItem, *mygo.Window) { a.openSettings() }},
		{Label: "切换中文 / 英文", Click: func(*mygo.MenuItem, *mygo.Window) {
			go a.client.Call(op("toggle_mode"))
		}},
		{Label: "中文 / 英文标点", Click: func(*mygo.MenuItem, *mygo.Window) {
			go a.client.Call(command("toggle_punct"))
		}},
		{Label: "简体 / 繁体", Click: func(*mygo.MenuItem, *mygo.Window) {
			go a.client.Call(command("toggle_traditional"))
		}},
		{Label: "显示 / 隐藏悬浮状态栏", Click: func(*mygo.MenuItem, *mygo.Window) {
			go a.client.Call(command("toggle_status_bar"))
		}},
		mygo.Separator(),
		{Label: "重新载入设置和词库", Click: func(*mygo.MenuItem, *mygo.Window) {
			go a.client.Call(op("reload"))
		}},
		mygo.Separator(),
		{Label: "关闭托盘图标", Click: func(*mygo.MenuItem, *mygo.Window) { mygo.App.Quit() }},
	})
	if _, err := mygo.NewTray(mygo.TrayOptions{
		ID:      "freetype",
		Icon:    icon,
		ToolTip: "FreeType 输入法",
		Menu:    menu,
	}); err != nil {
		log.Println("tray:", err)
	}
}

func main() {
	background := slices.Contains(os.Args[1:], "--background")
	if !mygo.App.RequestSingleInstanceLock() {
		return
	}
	client, err := newPipeClient()
	if err != nil {
		log.Fatal(err)
	}
	a := &app{client: client}
	mygo.App.OnSecondInstance(func(args []string, workingDir string) { a.openSettings() })
	// 关掉设置窗口后托盘图标还在
	mygo.App.OnWindowAllClosed(func() {})
	mygo.App.WhenReady(func() {
		a.resources, _ = mygo.App.Path(mygo.PathResources)
		go func() {
			if err := ensureServer(a.client, a.resources, 10*time.Second); err != nil {
				log.Println(err)
			}
		}()
		a.makeTray()
		if !background {
			a.openSettings()
		}
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
