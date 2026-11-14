package main

import (
	"errors"
	"log"
	"os"
	"slices"

	"github.com/egoist/mygo"
)

// mygo 的安装完成页会无参数运行程序。安装器的进程环境标记让这次启动也能
// 遵守安装后显示设置的选项；转为参数后立即清除，避免影响程序启动的其他应用。
// 必须在获取单实例锁前调用，才能把标记转交给已运行的 FreeType。
func consumeInstallerLaunch(args []string) ([]string, error) {
	if os.Getenv("FREETYPE_INSTALL_COMPLETED") != "1" {
		return args, nil
	}
	if err := os.Unsetenv("FREETYPE_INSTALL_COMPLETED"); err != nil {
		return nil, err
	}
	if !slices.Contains(args, "--after-install") {
		args = append(args, "--after-install")
	}
	return args, nil
}

// 安装完成的启动读取已保存的选项；开始菜单和开机后台启动保持各自的行为。
func shouldOpenSettings(args []string, backend Backend) (bool, error) {
	if slices.Contains(args, "--after-install") {
		response, err := backend.Call(op("get_config"))
		if err != nil {
			return false, err
		}
		if response.Config == nil {
			return false, errors.New("服务未返回安装后显示设置页的选项")
		}
		return response.Config.ShowSettingsAfterUpdate, nil
	}
	return !slices.Contains(args, "--background"), nil
}

func (a *app) handleLaunch(args []string) {
	go func() {
		show, err := shouldOpenSettings(args, a.client)
		if err != nil {
			log.Println("读取安装后显示设置页的选项：", err)
			return
		}
		if show {
			mygo.RunOnMain(a.openSettings)
		}
	}()
}
