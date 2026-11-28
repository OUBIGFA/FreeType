// FreeType.exe：托盘图标和设置窗口。
//
// 开机时以 --background 启动，只放托盘图标并确保输入法服务（bin\FreeTypeServer.exe）在跑；
// 从开始菜单打开时显示设置窗口，再次主动打开会把设置窗口带到前面；
// 安装完成后的启动按用户设置决定是否显示窗口，默认不显示。
package main

import (
	"context"
	"fmt"
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
	// 词库更新包：检查、下载和校验在设置程序里进行，服务进程下次启动时切换。
	dataUpdates *dataUpdates
	syncer      *autoSync
	syncError   string
	// 图像只在外观改变时载入；窗口关闭后仍保留，供托盘及新窗口继续使用。
	iconConfig         Config
	iconLogo, iconTray []byte
	// 系统品牌资源的更新串行进行；失败保留可见结果，重新选择或打开设置时可重试。
	systemBrandConfig   Config
	systemBrandTail     chan struct{}
	systemBrandRevision uint64
	systemBrandError    string
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
	s.config.BrandIcon, s.config.CustomBrandIcon = a.iconConfig.BrandIcon, a.iconConfig.CustomBrandIcon
	if a.syncer != nil {
		s.autoSync = a.syncer
		s.autoSyncStatus = a.syncer.snapshot()
	} else if a.syncError != "" {
		s.autoSyncStatus = autoSyncState{Failed: true, Message: a.syncError}
	}
	s.onTray = a.showTray
	s.onBrandIcon = a.applyBrandIcon
	s.notice = a.systemBrandError
	s.openIcon = func() (string, error) {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
			Parent: a.win, Title: "选择自定义品牌图标",
			Filters: []mygo.FileFilter{{Name: "图像文件", Extensions: []string{"png", "jpg", "jpeg", "webp"}}},
		})
		if err != nil {
			return "", err
		}
		if len(paths) == 0 {
			return "", nil
		}
		return paths[0], nil
	}
	s.openFolder = func(path string) { _ = exec.Command("explorer.exe", path).Start() }
	s.openURL = mygo.Shell.OpenExternal
	s.defaultTip = windowsDefaultTip
	s.openModel = func(title string) string {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{Parent: a.win, Title: title})
		if err != nil || len(paths) == 0 {
			return ""
		}
		return paths[0]
	}
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
	if a.dataUpdates != nil {
		s.dataUpdates, s.dataUpdate = a.dataUpdates, a.dataUpdates.snapshot()
		go func() { _ = a.dataUpdates.refresh() }()
	}
	s.openTable = func(title string) string {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
			Parent: a.win,
			Title:  title,
			Filters: []mygo.FileFilter{
				{Name: "TSV 词库", Extensions: []string{"tsv"}},
				{Name: "制表符分隔的文本词库", Extensions: []string{"txt"}},
			},
		})
		if err != nil || len(paths) == 0 {
			return ""
		}
		return paths[0]
	}
	s.openFile = func(title string) string {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
			Parent: a.win,
			Title:  title,
			// Rime 词典是 *.dict.yaml；别的输入法导出的文件扩展名不固定，留一项「所有文件」
			Filters: []mygo.FileFilter{
				{Name: "词库、短语或备份文件", Extensions: []string{"txt", "tsv", "scel", "yaml", "yml", "csv", "json"}},
				{Name: "所有文件", Extensions: []string{"*"}},
			},
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
	s.apply = func(fn func()) {
		if a.win == win {
			win.Update(fn)
		}
	}
	win.OnClosed(func() {
		s.flushPendingSave()
		s.cancelSyncPreview()
		a.win, a.settings = nil, nil
	})
	a.win, a.settings = win, s
	if err := a.setBrandIcon(a.iconConfig); err != nil {
		s.notice = "更新图标失败：" + err.Error()
	} else if err := win.SetIcon(a.iconLogo); err != nil {
		s.notice = "更新窗口图标失败：" + err.Error()
	}
	s.refresh()
}

// 直接替换现有窗口、托盘的图像，不重启应用或重建托盘，保留菜单和窗口状态。
func (a *app) setBrandIcon(config Config) error {
	if a.iconLogo != nil && config.BrandIcon == a.iconConfig.BrandIcon && config.CustomBrandIcon == a.iconConfig.CustomBrandIcon {
		return nil
	}
	logo, err := loadBrandIcon(a.resources, config, "logo.png")
	if err != nil {
		return err
	}
	tray, err := loadBrandIcon(a.resources, config, "tray.png")
	if err != nil {
		return err
	}
	if a.win != nil {
		if err := a.win.SetIcon(logo); err != nil {
			return fmt.Errorf("窗口图标：%w", err)
		}
	}
	if a.tray != nil {
		if err := a.tray.SetIcon(tray, false); err != nil {
			return fmt.Errorf("托盘图标：%w", err)
		}
	}
	a.iconConfig = Config{BrandIcon: config.BrandIcon, CustomBrandIcon: config.CustomBrandIcon}
	a.iconLogo, a.iconTray = logo, tray
	return nil
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
	if err := a.setBrandIcon(a.iconConfig); err != nil {
		log.Println("tray icon:", err)
		return
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
		Icon:    a.iconTray,
		ToolTip: "FreeType 输入法",
		Menu:    menu,
	})
	if err != nil {
		log.Println("tray:", err)
		return
	}
	// 左键单击或双击托盘图标都直接打开设置；窗口已打开时只切到前台，不重复创建。
	// 右键仍弹出菜单。mygo 在 Windows 上每次左键松开都会回调一次。
	tray.OnClick(a.openSettings)
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
	a := &app{client: client, iconConfig: defaultConfig()}
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
		dataTransport := netproxy.Transport().Clone()
		dataTransport.ResponseHeaderTimeout = 30 * time.Second
		if manager, err := newDataUpdates(a.client, &http.Client{Transport: dataTransport}, ""); err != nil {
			log.Println("initialize dictionary updates:", err)
		} else {
			a.dataUpdates = manager
			manager.changed = func(state dataUpdateState) {
				mygo.RunOnMain(func() {
					if a.settings != nil && a.win != nil {
						settings := a.settings
						a.win.Update(func() { settings.dataUpdate = state })
					}
				})
			}
		}
		go func() {
			err := ensureServer(a.client, a.resources, 60*time.Second)
			// 窗口可以先显示；载入结束后重新读状态，修正首次连接失败或未就绪的画面。
			mygo.RunOnMain(func() {
				if a.settings != nil {
					a.settings.refresh()
				}
			})
			if err != nil {
				log.Println(err)
				return
			}
			if a.updates != nil {
				a.updates.restorePending()
			}
			// 同步跟随应用存活，关闭设置窗口也继续；目录以服务实际使用的位置为准。
			if status, err := a.client.Call(op("status")); err == nil && status.Status != nil {
				syncer, syncErr := newAutoSync(status.Status.DataDir, a.client)
				mygo.RunOnMain(func() {
					if syncErr != nil {
						log.Println("initialize sync:", syncErr)
						a.syncError = syncErr.Error() + "；修正后请重新启动 FreeType 设置和托盘"
						if a.settings != nil {
							a.win.Update(func() { a.settings.autoSyncStatus = autoSyncState{Failed: true, Message: a.syncError} })
						}
						return
					}
					a.syncer = syncer
					syncer.changed = func(state autoSyncState) {
						mygo.RunOnMain(func() {
							if a.settings != nil && a.win != nil {
								s := a.settings
								a.win.Update(func() { s.autoSyncStatus = state })
							}
						})
					}
					syncer.restored = func(response Response) {
						mygo.RunOnMain(func() {
							if a.settings != nil && a.win != nil {
								s := a.settings
								a.win.Update(func() { s.applyRestored(response) })
							} else if response.Config != nil {
								if err := a.applyBrandIcon(*response.Config); err != nil {
									log.Println("restore brand icon:", err)
								}
								a.showTray(response.Config.TrayIcon)
								if a.updates != nil {
									a.updates.setAutomatic(response.Config.AutoUpdate)
								}
								if a.dataUpdates != nil {
									a.dataUpdates.setAutomatic(response.Config.DictionaryAutoUpdate)
								}
							}
						})
					}
					if a.settings != nil && a.win != nil {
						s := a.settings
						a.win.Update(func() { s.autoSync = syncer; s.autoSyncStatus = syncer.snapshot() })
					}
					syncer.start()
				})
			}
			// 安装后先等服务可读，再决定是否创建设置窗口，默认关闭时不会闪现。
			if afterInstall {
				a.handleLaunch(args)
			}
			config, err := a.client.Call(op("get_config"))
			if err != nil || config.Config == nil {
				return
			}
			// 服务确认启动后再读取图标，开机时服务尚未就绪也能恢复已保存的外观。
			loadedConfig := *config.Config
			mygo.RunOnMain(func() {
				if err := a.applyBrandIcon(loadedConfig); err != nil {
					log.Println("load brand icon:", err)
					if a.settings != nil {
						a.win.Update(func() { a.settings.notice = "读取图标失败：" + err.Error() })
					}
				}
				a.showTray(loadedConfig.TrayIcon)
			})
			if a.dataUpdates != nil {
				_ = a.dataUpdates.refresh()
				a.dataUpdates.setAutomatic(loadedConfig.DictionaryAutoUpdate)
			}
			if a.updates == nil {
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
	if a.dataUpdates != nil {
		a.dataUpdates.close()
	}
	if a.syncer != nil {
		a.syncer.close()
	}
}
