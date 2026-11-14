// FreeType.exe：托盘图标和设置窗口。
//
// 开机时以 --background 启动，只放托盘图标并确保输入法服务（bin\FreeTypeServer.exe）在跑；
// 从开始菜单打开时显示设置窗口，再次主动打开会把设置窗口带到前面；
// 安装完成后的启动按用户设置决定是否显示窗口，默认不显示。
package main

import (
	"context"
	"log"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/netproxy"
	"github.com/egoist/mygo/ui"

	// 语言目录：安装器和 mygo 自己的控件文字按系统语言显示（简体中文、繁体中文、英文）
	_ "github.com/oubigfa/freetype/windows/app/locales"
)

type app struct {
	client    Backend
	resources string
	win       *mygo.Window
	settings  *settings
	tray      *mygo.Tray
	updates   *updates
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
	s.onTray = a.showTray
	s.openFolder = func(path string) { _ = exec.Command("explorer.exe", path).Start() }
	s.openURL = mygo.Shell.OpenExternal
	if a.updates != nil {
		s.updateStatus = a.updates.snapshot()
		s.onAutomaticUpdate = a.updates.setAutomatic
		s.onCheckUpdates = func() {
			go func() {
				ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
				defer cancel()
				_ = a.updates.check(ctx) // The manager exposes errors as a visible state.
			}()
		}
		s.onUpdateNow = func() {
			go func() {
				executable, err := os.Executable()
				if err == nil {
					err = a.updates.updateNow(filepath.Dir(executable))
				}
				if err != nil {
					log.Println("update now:", err)
					return
				}
				mygo.RunOnMain(func() { mygo.App.Quit() })
			}()
		}
	} else {
		s.updateStatus.Message = "更新检查暂不可用，请重启 FreeType 后重试"
	}
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
			Filters: []mygo.FileFilter{{Name: "词库、短语或备份文件", Extensions: []string{"txt", "tsv", "scel", "csv", "json"}}},
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

// showTray 按设置建或拿掉托盘图标（在主线程上）。
func (a *app) showTray(show bool) {
	if show && a.tray == nil {
		a.makeTray()
	} else if !show && a.tray != nil {
		a.tray.Destroy()
		a.tray = nil
	}
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
		{Label: "关闭设置和托盘", Click: func(*mygo.MenuItem, *mygo.Window) { mygo.App.Quit() }},
	})
	tray, err := mygo.NewTray(mygo.TrayOptions{
		ID:      "freetype",
		Icon:    icon,
		ToolTip: "FreeType 输入法",
		Menu:    menu,
	})
	if err != nil {
		log.Println("tray:", err)
		return
	}
	a.tray = tray
}

func main() {
	args, err := consumeInstallerLaunch(os.Args[1:])
	if err != nil {
		log.Fatal(err)
	}
	os.Args = append(os.Args[:1], args...)
	afterInstall := slices.Contains(args, "--after-install")
	if !mygo.App.RequestSingleInstanceLock() {
		return
	}
	client, err := newPipeClient()
	if err != nil {
		log.Fatal(err)
	}
	a := &app{client: client}
	mygo.App.OnSecondInstance(func(args []string, workingDir string) { a.handleLaunch(args) })
	// 关掉设置窗口后托盘图标还在
	mygo.App.OnWindowAllClosed(func() {})
	mygo.App.WhenReady(func() {
		a.resources, _ = mygo.App.Path(mygo.PathResources)
		if dir, err := mygo.App.Path(mygo.PathUserData); err == nil {
			transport := netproxy.Transport().Clone()
			transport.ResponseHeaderTimeout = 30 * time.Second
			a.updates, err = newUpdates(dir, mygo.App.Version(), &http.Client{Transport: transport}, "")
			if err != nil {
				log.Println("initialize updates:", err)
			} else {
				a.updates.changed = func(state updateState) {
					mygo.RunOnMain(func() {
						if a.settings != nil && a.win != nil {
							settings := a.settings
							a.win.Update(func() { settings.updateStatus = state })
						}
					})
				}
			}
		} else {
			log.Println("update data directory:", err)
		}
		go func() {
			if err := ensureServer(a.client, a.resources, 10*time.Second); err != nil {
				log.Println(err)
				return
			}
			// 安装后先等服务可读，再决定是否创建设置窗口，默认关闭时不会闪现。
			if afterInstall {
				a.handleLaunch(args)
			}
			config, err := a.client.Call(op("get_config"))
			if err != nil || config.Config == nil || a.updates == nil {
				return
			}
			if config.Config.AutoUpdate && a.updates.snapshot().Ready {
				executable, err := os.Executable()
				if err == nil {
					err = a.updates.install(filepath.Dir(executable))
				}
				if err == nil {
					mygo.RunOnMain(func() { mygo.App.Quit() })
					return
				}
				log.Println("apply downloaded update:", err)
			}
			a.updates.setAutomatic(config.Config.AutoUpdate)
		}()
		// 先放托盘图标；读到设置里关掉了再拿掉
		a.makeTray()
		go func() {
			if resp, err := a.client.Call(op("get_config")); err == nil && resp.Config != nil && !resp.Config.TrayIcon {
				show := resp.Config.TrayIcon
				mygo.RunOnMain(func() { a.showTray(show) })
			}
		}()
		if !afterInstall {
			a.handleLaunch(args)
		}
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
	if a.updates != nil {
		a.updates.close()
	}
}
