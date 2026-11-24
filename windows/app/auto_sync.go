package main

import (
	"cmp"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// 数据版本与上传时间分开。相同毫秒由来源设备、摘要固定排序，所有设备得到同一结果。
type syncVersion struct {
	Modified int64  `json:"modified_unix_ms"`
	Origin   string `json:"origin_device"`
	Hash     string `json:"sha256"`
}

type syncCheckpoint struct {
	Target  string      `json:"target"`
	Version syncVersion `json:"version"`
	// 恢复前落盘；进程中断后可认出已恢复的内容，避免把恢复时间当成新编辑。
	Pending *syncVersion `json:"pending,omitempty"`
}

type autoSyncState struct {
	Message    string         `json:"message"`
	Failed     bool           `json:"failed"`
	Busy       bool           `json:"-"`
	Last       time.Time      `json:"last"`
	Next       time.Time      `json:"next"`
	Day        string         `json:"day"`
	Count      int            `json:"count"`
	Retries    int            `json:"retries"`
	Checkpoint syncCheckpoint `json:"checkpoint"`
}

func validateSyncPlan(c syncSettings) error {
	if c.Schedule != "interval" && c.Schedule != "daily" {
		return errors.New("请选择按间隔或每天定时同步")
	}
	if c.IntervalMinutes < 1 || c.IntervalMinutes > 1440 {
		return errors.New("同步间隔应为 1 至 1440 分钟")
	}
	if _, err := time.Parse("15:04", c.DailyTime); err != nil {
		return errors.New("每天同步时间应为 HH:MM，例如 21:00")
	}
	if c.DailyLimit < 0 || c.DailyLimit > 1440 {
		return errors.New("每日次数应为 0 至 1440，0 表示不限")
	}
	if c.RetryLimit < 0 || c.RetryLimit > 10 {
		return errors.New("失败重试次数应为 0 至 10")
	}
	if c.KeepSnapshots < 1 || c.KeepSnapshots > 100 {
		return errors.New("远端备份保留数应为 1 至 100")
	}
	return nil
}

func nextSyncTime(c syncSettings, now time.Time) time.Time {
	if c.Schedule == "daily" {
		clock, _ := time.Parse("15:04", c.DailyTime)
		next := time.Date(now.Year(), now.Month(), now.Day(), clock.Hour(), clock.Minute(), 0, 0, now.Location())
		if !next.After(now) {
			next = next.AddDate(0, 0, 1)
		}
		return next
	}
	return now.Add(time.Duration(c.IntervalMinutes) * time.Minute)
}

func compareSyncVersions(a, b syncVersion) int {
	if n := cmp.Compare(a.Modified, b.Modified); n != 0 {
		return n
	}
	if n := strings.Compare(a.Origin, b.Origin); n != 0 {
		return n
	}
	return strings.Compare(a.Hash, b.Hash)
}

func deviceVersion(d syncDevice) syncVersion {
	stamp, origin := d.Modified, d.Origin
	// 老版快照没有数据时间，沿用其备份时间以便平滑迁移。
	if stamp == 0 {
		stamp = d.Uploaded * 1000
	}
	if origin == "" {
		origin = d.ID
	}
	return syncVersion{Modified: stamp, Origin: origin, Hash: d.Hash}
}

func newestSyncDevice(devices []syncDevice) syncDevice {
	var newest syncDevice
	for _, d := range devices {
		if d.Revision != "" && (newest.ID == "" || compareSyncVersions(deviceVersion(d), deviceVersion(newest)) > 0) {
			newest = d
		}
	}
	return newest
}

func localSyncVersion(cp *syncCheckpoint, target, hash string, modified int64, id string) syncVersion {
	if cp.Target != target {
		*cp = syncCheckpoint{Target: target}
	}
	if cp.Pending != nil && cp.Pending.Hash == hash {
		cp.Version, cp.Pending = *cp.Pending, nil
	}
	cp.Pending = nil
	if cp.Version.Hash == hash {
		return cp.Version
	}
	// 在已知远端版本之后编辑，即使系统时间回拨也必须成为后继版本。
	if modified <= cp.Version.Modified {
		modified = cp.Version.Modified + 1
	}
	return syncVersion{Modified: modified, Origin: id, Hash: hash}
}

func snapshotHash(path string) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer file.Close()
	h := sha256.New()
	if _, err := io.Copy(h, file); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

func syncTarget(c syncSettings) string {
	// 不存地址或凭据；只用摘要判定是否仍为同一同步目标。
	var identity any
	if c.Provider == "s3" {
		identity = []string{c.Provider, c.S3Endpoint, c.S3Region, c.S3Bucket, c.S3Prefix, c.S3AccessKey}
	} else {
		identity = []string{c.Provider, c.WebDAVURL, c.WebDAVUser}
	}
	raw, _ := json.Marshal(identity)
	h := sha256.Sum256(raw)
	return hex.EncodeToString(h[:])
}

// autoSync 属于托盘应用，不属于设置窗口。gate 也给手动动作使用，防止同时读写快照。
type autoSync struct {
	mu        sync.Mutex
	gate      sync.Mutex
	dir       string
	backend   Backend
	config    syncSettings
	state     autoSyncState
	openStore func(syncSettings) (remoteStore, error)
	changed   func(autoSyncState)
	restored  func(Response)
	stop      chan struct{}
	closeOnce sync.Once
}

func newAutoSync(dir string, backend Backend) (*autoSync, error) {
	c, err := loadSyncSettings(dir)
	if err != nil {
		return nil, err
	}
	if err := validateSyncPlan(c); err != nil && c.Enabled && c.Automatic {
		return nil, err
	}
	a := &autoSync{dir: dir, backend: backend, config: c, openStore: openRemoteStore, stop: make(chan struct{})}
	raw, err := os.ReadFile(filepath.Join(dir, "sync-state.json"))
	if err == nil {
		err = json.Unmarshal(raw, &a.state)
	}
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, fmt.Errorf("读取自动同步状态失败：%w", err)
	}
	// 没有配置时不写 sync.json，避免与第一次打开设置页生成设备标识竞争。
	if c.Enabled && c.Automatic {
		if c.SyncOnStart {
			a.state.Next = time.Now().Add(15 * time.Second)
		} else if a.state.Next.IsZero() {
			a.state.Next = nextSyncTime(c, time.Now())
		}
	}
	return a, nil
}

func (a *autoSync) snapshot() autoSyncState { a.mu.Lock(); defer a.mu.Unlock(); return a.state }

func (a *autoSync) publish() {
	if a.changed != nil {
		a.changed(a.snapshot())
	}
}

// 调用时持有 mu；状态与配额先写临时文件，再整体替换。
func (a *autoSync) saveLocked() error {
	raw, err := json.MarshalIndent(a.state, "", "  ")
	if err != nil {
		return err
	}
	if err = os.MkdirAll(a.dir, 0700); err != nil {
		return err
	}
	path := filepath.Join(a.dir, "sync-state.json")
	if err = os.WriteFile(path+".tmp", append(raw, '\n'), 0600); err != nil {
		return err
	}
	return os.Rename(path+".tmp", path)
}

func (a *autoSync) configure(c syncSettings) {
	a.mu.Lock()
	if a.config != c {
		a.config = c
		a.state.Retries = 0
		a.state.Next = nextSyncTime(c, time.Now())
		if !c.Enabled || !c.Automatic {
			a.state.Next = time.Time{}
		}
		if err := a.saveLocked(); err != nil {
			a.state.Message = "保存同步计划状态失败：" + err.Error()
			a.state.Failed = true
		}
	}
	a.mu.Unlock()
	a.publish()
}

func (a *autoSync) start() {
	go func() {
		timer := time.NewTicker(time.Second)
		defer timer.Stop()
		for {
			select {
			case <-a.stop:
				return
			case now := <-timer.C:
				a.run(now, false)
			}
		}
	}()
}

func (a *autoSync) close() { a.closeOnce.Do(func() { close(a.stop) }) }

// run 在后台调用；一次轮次含失败重试，每次网络尝试都计入每日上限。
func (a *autoSync) run(now time.Time, manual bool) {
	if !a.gate.TryLock() {
		return
	}
	defer a.gate.Unlock()
	a.mu.Lock()
	c := a.config
	if !c.Enabled || (!manual && (!c.Automatic || a.state.Next.IsZero() || now.Before(a.state.Next))) {
		a.mu.Unlock()
		return
	}
	if a.state.Day != now.Format("2006-01-02") {
		a.state.Day = now.Format("2006-01-02")
		a.state.Count = 0
	}
	if !manual && c.DailyLimit > 0 && a.state.Count >= c.DailyLimit {
		a.state.Next = time.Date(now.Year(), now.Month(), now.Day()+1, 0, 0, 0, 0, now.Location())
		if c.Schedule == "daily" {
			a.state.Next = nextSyncTime(c, now)
		}
		a.state.Message = "今日自动同步次数已用完"
		a.mu.Unlock()
		a.publish()
		return
	}
	if !manual {
		a.state.Count++
	}
	a.state.Busy = true
	a.state.Message = "正在比较并同步最新备份"
	a.state.Failed = false
	err := a.saveLocked()
	a.mu.Unlock()
	a.publish()
	var message string
	started := time.Now()
	if err == nil {
		message, err = a.cycle(c)
	}
	a.mu.Lock()
	// 按完成时间安排下一次，慢网络或休眠恢复后不会积压连续同步。
	finished := now.Add(time.Since(started))
	a.state.Busy = false
	a.state.Last = now
	if err != nil {
		a.state.Failed = true
		a.state.Message = err.Error()
		if a.config == c && c.Automatic && a.state.Retries < c.RetryLimit {
			a.state.Next = finished.Add(time.Minute * time.Duration(1<<a.state.Retries))
			a.state.Retries++
		} else {
			a.state.Retries = 0
			a.state.Next = nextSyncTime(a.config, finished)
		}
	} else {
		a.state.Message = message
		a.state.Retries = 0
		a.state.Next = nextSyncTime(a.config, finished)
	}
	if !a.config.Enabled || !a.config.Automatic {
		a.state.Next = time.Time{}
	}
	if saveErr := a.saveLocked(); saveErr != nil {
		a.state.Failed = true
		a.state.Message = "保存同步状态失败：" + saveErr.Error()
	}
	a.mu.Unlock()
	a.publish()
}

func (a *autoSync) stillConfigured(c syncSettings) bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	select {
	case <-a.stop:
		return false
	default:
	}
	return a.config == c && c.Enabled
}

func (a *autoSync) cycle(c syncSettings) (string, error) {
	return a.cycleWith(c, func() (remoteStore, error) {
		store, err := a.openStore(c)
		if err != nil {
			return nil, err
		}
		// 后台请求必须有结束时间，否则一次断网会永久占用同步队列。
		switch s := store.(type) {
		case *webdavStore:
			s.client.Timeout = 2 * time.Minute
		case *s3Store:
			s.client.Timeout = 2 * time.Minute
		}
		// 超时按具体的存储设好以后，再套上加密层。
		if c.Encrypt && c.EncryptPassword == "" {
			return nil, errNoPassword
		}
		return withBackupPassword(store, c), nil
	})
}

func (a *autoSync) cycleWith(c syncSettings, connect func() (remoteStore, error)) (string, error) {
	if err := validateSyncPlan(c); err != nil {
		return "", err
	}
	workRoot := filepath.Join(a.dir, "_temp")
	if err := os.MkdirAll(workRoot, 0700); err != nil {
		return "", err
	}
	work, err := os.MkdirTemp(workRoot, "sync-")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(work)
	local := filepath.Join(work, "local.json")
	resp, err := a.backend.Call(op("export_sync_settings", "path", local))
	if err != nil {
		return "", fmt.Errorf("生成同步备份失败：%w", err)
	}
	hash, err := snapshotHash(local)
	if err != nil {
		return "", err
	}
	a.mu.Lock()
	version := localSyncVersion(&a.state.Checkpoint, syncTarget(c), hash, resp.ModifiedUnixMS, c.DeviceID)
	// 失败或重启不会让本次本机版本失去身份。
	a.state.Checkpoint.Version = version
	err = a.saveLocked()
	a.mu.Unlock()
	if err != nil {
		return "", err
	}
	store, err := connect()
	if err != nil {
		return "", err
	}
	devices, err := readDevices(store, true)
	if err != nil {
		return "", err
	}
	if resp.SyncFormat != 1 {
		return "", errors.New("正在运行的服务不支持逐项同步；请重启 FreeType 后再试")
	}
	return a.cycleRecords(c, store, devices, local, work, version)
}
