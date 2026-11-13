package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/Masterminds/semver/v3"
	selfupdate "github.com/creativeprojects/go-selfupdate"
)

type updateState struct {
	Version, Message, Notes string
	Busy, Ready             bool
}

type pendingUpdate struct {
	Version string `json:"version"`
	SHA256  string `json:"sha256"`
}

// The library owns version selection, downloading, SHA validation and atomic
// file replacement. FreeType schedules checks and hands the verified NSIS
// installer to its existing complete-component installation path.
type updates struct {
	directory, current string
	source             *githubUpdates
	updater            *selfupdate.Updater
	mu                 sync.Mutex
	operation          sync.Mutex
	state              updateState
	release            *selfupdate.Release
	automatic          bool
	timer              *time.Timer
	autoCancel         context.CancelFunc
	autoGeneration     uint64
	changed            func(updateState)
	launch             func(path, directory string) error
}

func newUpdates(dataDir, current string, client *http.Client, apiBase string) (*updates, error) {
	if _, err := semver.NewVersion(current); err != nil {
		return nil, err
	}
	source, err := newUpdateSource(client, apiBase)
	if err != nil {
		return nil, err
	}
	updater, err := selfupdate.NewUpdater(selfupdate.Config{
		Source: source, Validator: &selfupdate.SHAValidator{},
		OS: "windows", Arch: "amd64",
		Filters: []string{`^freetype-[0-9]+\.[0-9]+\.[0-9]+-setup\.exe$`},
	})
	if err != nil {
		return nil, err
	}
	manager := &updates{
		directory: filepath.Join(dataDir, "updates"), current: current, source: source, updater: updater,
		state: updateState{Message: "点击“检查更新”查看可用版本"}, launch: launchUpdateInstaller,
	}
	if pending, err := manager.readPending(); err == nil {
		manager.state.Version = pending.Version
		if err := manager.verifyPending(); err == nil {
			manager.state.Ready, manager.state.Message = true, "更新已下载，可立即更新；开启自动更新后，下次启动时安装"
		} else {
			manager.state.Message = "已下载的更新未通过校验，请重新检查并下载"
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		manager.state.Message = "无法读取待更新记录，请重新检查并下载"
	}
	return manager, nil
}

func (manager *updates) installerPath() string {
	return filepath.Join(manager.directory, "FreeType-Update-Setup.exe")
}
func (manager *updates) pendingPath() string { return filepath.Join(manager.directory, "pending.json") }

func (manager *updates) snapshot() updateState {
	manager.mu.Lock()
	defer manager.mu.Unlock()
	return manager.state
}

func (manager *updates) change(fn func(*updateState)) {
	manager.mu.Lock()
	fn(&manager.state)
	state, changed := manager.state, manager.changed
	manager.mu.Unlock()
	if changed != nil {
		changed(state)
	}
}

func (manager *updates) begin(message string) bool {
	manager.mu.Lock()
	if manager.state.Busy {
		manager.mu.Unlock()
		return false
	}
	manager.state.Busy, manager.state.Message = true, message
	state, changed := manager.state, manager.changed
	manager.mu.Unlock()
	if changed != nil {
		changed(state)
	}
	return true
}

func (manager *updates) check(ctx context.Context) error {
	manager.operation.Lock()
	defer manager.operation.Unlock()
	if err := ctx.Err(); err != nil {
		return err
	}
	if !manager.begin("正在检查更新…") {
		return errors.New("更新任务正在进行")
	}
	release, found, err := manager.updater.DetectLatest(ctx, selfupdate.ParseSlug("OUBIGFA/FreeType"))
	manager.mu.Lock()
	if err == nil {
		manager.release = release
	}
	manager.mu.Unlock()
	manager.change(func(state *updateState) {
		state.Busy = false
		switch {
		case errors.Is(err, context.Canceled):
			state.Message = "已停止本次更新检查"
		case err != nil:
			state.Message = "检查更新失败，请检查网络后重试"
		case !found:
			if !state.Ready {
				state.Version, state.Notes = "", ""
			}
			state.Message = "尚无已发布的 Windows 更新安装器"
		case !release.GreaterThan(manager.current):
			if state.Ready {
				state.Message = "当前更新尚未完成，可继续安装"
			} else {
				state.Version, state.Notes = "", ""
				state.Message = "已是最新版本"
			}
		default:
			if state.Version != release.Version() {
				state.Ready = false
			}
			state.Version, state.Notes = release.Version(), release.ReleaseNotes
			state.Message = "可更新到 " + release.Version()
		}
	})
	if err != nil {
		log.Println("check updates:", err)
	}
	return err
}

func (manager *updates) download(ctx context.Context) (err error) {
	manager.operation.Lock()
	defer manager.operation.Unlock()
	if err = ctx.Err(); err != nil {
		return err
	}
	manager.mu.Lock()
	release, ready := manager.release, manager.state.Ready
	manager.mu.Unlock()
	if ready {
		return manager.verifyPending()
	}
	if release == nil || !release.GreaterThan(manager.current) {
		return errors.New("没有可下载的更新")
	}
	if !manager.begin("正在后台下载并校验更新…") {
		return errors.New("更新任务正在进行")
	}
	defer func() {
		manager.change(func(state *updateState) {
			state.Busy = false
			if errors.Is(err, context.Canceled) {
				state.Message = "已停止本次下载"
			} else if err != nil {
				state.Message = "下载或校验更新失败，请重新尝试"
			} else {
				state.Ready, state.Version = true, release.Version()
				state.Message = "更新已下载，可立即更新；开启自动更新后，下次启动时安装"
			}
		})
		if err != nil {
			log.Println("download update:", err)
		}
	}()
	if err = os.MkdirAll(manager.directory, 0o700); err != nil {
		return err
	}
	// UpdateTo replaces an existing target. Seed only a new private staging
	// file, so a failed download keeps a previously verified installer intact.
	seed, createErr := os.OpenFile(manager.installerPath(), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o700)
	if createErr == nil {
		seed.Close()
	} else if !errors.Is(createErr, os.ErrExist) {
		return createErr
	}
	if err = manager.updater.UpdateTo(ctx, release, manager.installerPath()); err != nil {
		return err
	}
	manager.source.mu.Lock()
	digest := manager.source.digests[release.AssetID]
	manager.source.mu.Unlock()
	data, err := json.Marshal(pendingUpdate{Version: release.Version(), SHA256: digest})
	if err != nil {
		return err
	}
	if err = os.WriteFile(manager.pendingPath()+".tmp", data, 0o600); err != nil {
		return err
	}
	return os.Rename(manager.pendingPath()+".tmp", manager.pendingPath())
}

func (manager *updates) readPending() (pending pendingUpdate, err error) {
	data, err := os.ReadFile(manager.pendingPath())
	if err != nil {
		return pending, err
	}
	if err = json.Unmarshal(data, &pending); err != nil {
		return pending, err
	}
	version, err := semver.StrictNewVersion(pending.Version)
	if err != nil {
		return pending, err
	}
	current, _ := semver.NewVersion(manager.current)
	// NSIS keeps this record if it copied the new app but TSF registration
	// failed. Permit that same-version retry, while never installing an older one.
	if version.LessThan(current) {
		return pending, errors.New("待更新版本低于当前版本")
	}
	if len(pending.SHA256) != 64 {
		return pending, errors.New("待更新校验值无效")
	}
	return pending, nil
}

func (manager *updates) verifyPending() error {
	pending, err := manager.readPending()
	if err != nil {
		return err
	}
	data, err := os.ReadFile(manager.installerPath())
	if err != nil {
		return err
	}
	return (&selfupdate.SHAValidator{}).Validate("FreeType-Update-Setup.exe", data, []byte(pending.SHA256))
}

func (manager *updates) install(directory string) error {
	manager.operation.Lock()
	defer manager.operation.Unlock()
	if !manager.begin("正在启动更新安装器…") {
		return errors.New("更新任务正在进行")
	}
	if err := manager.verifyPending(); err != nil {
		manager.change(func(state *updateState) {
			state.Busy = false
			state.Ready = false
			state.Message = "安装器未通过校验，请重新下载更新"
		})
		return err
	}
	if err := manager.launch(manager.installerPath(), directory); err != nil {
		manager.change(func(state *updateState) {
			state.Busy = false
			state.Message = "无法启动更新安装器，请重试"
		})
		return err
	}
	// The NSIS hook clears pending.json only after TSF registration succeeds.
	manager.change(func(state *updateState) { state.Busy = true; state.Message = "更新安装器已启动" })
	return nil
}

func (manager *updates) setAutomatic(on bool) {
	manager.mu.Lock()
	if manager.automatic == on {
		manager.mu.Unlock()
		return
	}
	manager.automatic = on
	manager.autoGeneration++
	generation := manager.autoGeneration
	if manager.timer != nil {
		manager.timer.Stop()
		manager.timer = nil
	}
	if !on && manager.autoCancel != nil {
		manager.autoCancel()
	}
	manager.mu.Unlock()
	if on {
		go manager.automaticCheck(generation)
	}
}

func (manager *updates) automaticCheck(generation uint64) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Minute)
	defer cancel()
	manager.mu.Lock()
	if !manager.automatic || manager.autoGeneration != generation {
		manager.mu.Unlock()
		return
	}
	manager.autoCancel = cancel
	manager.mu.Unlock()
	if err := manager.check(ctx); err == nil {
		state := manager.snapshot()
		if state.Version != "" && !state.Ready {
			_ = manager.download(ctx) // Download failures stay visible in the state and log.
		}
	}
	manager.mu.Lock()
	if manager.autoGeneration == generation {
		manager.autoCancel = nil
		if manager.automatic {
			manager.timer = time.AfterFunc(24*time.Hour, func() { manager.automaticCheck(generation) })
		}
	}
	manager.mu.Unlock()
}

func (manager *updates) close() { manager.setAutomatic(false) }

func (manager *updates) updateNow(directory string) error {
	if !manager.snapshot().Ready {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Minute)
		defer cancel()
		manager.mu.Lock()
		needsCheck := manager.release == nil
		manager.mu.Unlock()
		if needsCheck {
			if err := manager.check(ctx); err != nil {
				return err
			}
		}
		if err := manager.download(ctx); err != nil {
			return err
		}
	}
	return manager.install(directory)
}

func (state updateState) available() bool { return state.Ready || state.Version != "" }
