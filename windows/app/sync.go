package main

// 数据同步：把「全部设置」快照手动上传到用户自己的 WebDAV 或 S3 兼容存储，
// 再在另一台电脑（或本机）上合并恢复。数据只发往用户填写的地址。
//
// 远端布局：备份直接放在 FreeType 内，时间和电脑名用于辨认。
//
//	<根>/FreeType/sync-年月日时分秒-电脑名.json
//
// 设备信息放在每份备份的首行，不单独生成索引文件或设备目录。
//
// 同步的地址、账号和设备标识存在用户数据目录的 sync.json，不进快照、不被恢复覆盖；
// 密码和密钥用 Windows DPAPI（当前用户）加密后才写盘。

import (
	"bytes"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
)

// 每台设备在远端保留几份快照。
const keepSnapshots = 5

// syncSettings 是 sync.json 的内容。密码、密钥在内存里是明文，写盘前加密。
type syncSettings struct {
	Enabled  bool   `json:"enabled"`
	Provider string `json:"provider"` // webdav 或 s3

	WebDAVURL      string `json:"webdav_url"`
	WebDAVUser     string `json:"webdav_user"`
	WebDAVPassword string `json:"webdav_password"`

	S3Endpoint  string `json:"s3_endpoint"`
	S3Region    string `json:"s3_region"`
	S3Bucket    string `json:"s3_bucket"`
	S3Prefix    string `json:"s3_prefix"`
	S3AccessKey string `json:"s3_access_key"`
	S3SecretKey string `json:"s3_secret_key"`
	S3PathStyle bool   `json:"s3_path_style"`

	// 计划与凭据一样只属于本机，不随远端快照恢复。
	Automatic       bool   `json:"automatic"`
	Schedule        string `json:"schedule"`
	IntervalMinutes int    `json:"interval_minutes"`
	DailyTime       string `json:"daily_time"`
	SyncOnStart     bool   `json:"sync_on_start"`
	DailyLimit      int    `json:"daily_limit"`
	RetryLimit      int    `json:"retry_limit"`
	KeepSnapshots   int    `json:"keep_snapshots"`

	// 这台设备在远端的名字：32 位小写十六进制，生成一次后一直用（覆盖安装不变）。
	DeviceID string `json:"device_id"`

	// 云端备份加密（默认关闭）：上传的备份用这个密码加密，见 backup_crypto.go。
	// 密码和网盘密码一样只在本机、加密保存。
	Encrypt         bool   `json:"encrypt"`
	EncryptPassword string `json:"encrypt_password"`
}

// secrets 是写盘前要用系统的数据保护加密的字段。
func (c *syncSettings) secrets() []*string {
	return []*string{&c.WebDAVPassword, &c.S3SecretKey, &c.EncryptPassword}
}

var (
	deviceIDPattern = regexp.MustCompile(`^[0-9a-f]{32}$`)
	revisionPattern = regexp.MustCompile(`^(\d{1,19})-(\d{1,10})$`)
)

// 加密后的值带这个前缀；没有前缀的当明文读（手工改过文件）。
const protectedPrefix = "dpapi:"

func newDeviceID() (string, error) {
	raw := make([]byte, 16)
	if _, err := rand.Read(raw); err != nil {
		return "", err
	}
	return hex.EncodeToString(raw), nil
}

// loadSyncSettings 读 sync.json；没有文件就是默认值。设备标识不对或没有时生成一个新的。
func loadSyncSettings(dir string) (syncSettings, error) {
	c := syncSettings{Provider: "webdav", S3PathStyle: true, Schedule: "interval",
		IntervalMinutes: 30, DailyTime: "21:00", SyncOnStart: true, RetryLimit: 3, KeepSnapshots: keepSnapshots}
	data, err := os.ReadFile(filepath.Join(dir, "sync.json"))
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return c, fmt.Errorf("读取同步设置失败：%w", err)
	}
	if err == nil {
		if err := json.Unmarshal(bytes.TrimPrefix(data, []byte("\xef\xbb\xbf")), &c); err != nil {
			return c, fmt.Errorf("同步设置文件 sync.json 格式不正确：%w", err)
		}
	}
	for _, secret := range c.secrets() {
		if text, ok := strings.CutPrefix(*secret, protectedPrefix); ok {
			plain, err := unprotect(text)
			if err != nil {
				// 换了 Windows 账号或电脑：密文解不开，请用户重新填
				*secret = ""
				continue
			}
			*secret = plain
		}
	}
	if c.Provider != "s3" {
		c.Provider = "webdav"
	}
	if !deviceIDPattern.MatchString(c.DeviceID) {
		if c.DeviceID, err = newDeviceID(); err != nil {
			return c, fmt.Errorf("生成设备标识失败：%w", err)
		}
	}
	return c, nil
}

// saveSyncSettings 写 sync.json（先写临时文件再改名）。
func saveSyncSettings(dir string, c syncSettings) error {
	for _, secret := range c.secrets() {
		if *secret == "" {
			continue
		}
		sealed, err := protect(*secret)
		if err != nil {
			return fmt.Errorf("加密保存密码失败：%w", err)
		}
		*secret = protectedPrefix + sealed
	}
	data, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("保存同步设置失败：%w", err)
	}
	path := filepath.Join(dir, "sync.json")
	temp := path + ".tmp"
	if err := os.WriteFile(temp, append(data, '\n'), 0o600); err != nil {
		return fmt.Errorf("保存同步设置失败：%w", err)
	}
	if err := os.Rename(temp, path); err != nil {
		os.Remove(temp)
		return fmt.Errorf("保存同步设置失败：%w", err)
	}
	return nil
}

// openRemoteStore 按设置连到远端。
func openRemoteStore(c syncSettings) (remoteStore, error) {
	if c.Provider == "s3" {
		s, err := newS3Store(c)
		if err != nil {
			return nil, err
		}
		if len(s.prefix) == 0 || s.prefix[len(s.prefix)-1] != "FreeType" {
			old := *s
			s.legacy = &old
			s.prefix = append(append([]string{}, s.prefix...), "FreeType")
		}
		return s, nil
	}
	w, err := newWebDAVStore(c.WebDAVURL, c.WebDAVUser, c.WebDAVPassword)
	if err != nil {
		return nil, err
	}
	if !strings.HasSuffix(w.base.Path, "/FreeType") {
		old := *w
		w.legacy = &old
		base := *w.base
		base.Path += "/FreeType"
		w.base = &base
	}
	return w, nil
}

// syncDevice 是备份内的设备与版本信息；兼容读取旧版独立设备索引。
type syncDevice struct {
	ID       string `json:"device_id"`
	Hostname string `json:"hostname"`
	Revision string `json:"revision"`
	Uploaded int64  `json:"uploaded_unix"`
	Size     int64  `json:"size"`
	Modified int64  `json:"modified_unix_ms,omitempty"`
	Origin   string `json:"origin_device,omitempty"`
	Hash     string `json:"sha256,omitempty"`
	// 旧目录只读；恢复必须使用列举这条记录的同一位置。
	legacy bool
	flat   bool
}

func (d syncDevice) snapshotKey() string {
	if d.flat {
		return d.Revision + ".json"
	}
	return snapshotKey(d.ID, d.Revision)
}

func (d syncDevice) snapshotStore(fallback remoteStore) remoteStore {
	if d.legacy {
		if source, ok := fallback.(interface{ legacyStore() remoteStore }); ok && source.legacyStore() != nil {
			return source.legacyStore()
		}
	}
	return fallback
}

func deviceInfoKey(id string) string         { return "devices/" + id + ".json" }
func snapshotKey(id, revision string) string { return "devices/" + id + "/" + revision + ".json" }

// revisionOrder 把修订号拆成 (秒级时间戳, 序号)；兼容旧版以进程号作序号。
func revisionOrder(revision string) (int64, int64, bool) {
	if stamp, ok := strings.CutPrefix(revision, "sync-"); ok {
		if len(stamp) < 14 {
			return 0, 0, false
		}
		if len(stamp) > 14 && (stamp[14] != '-' || len(stamp) == 15 || strings.ContainsAny(stamp[15:], `<>:"/\|?*`) || strings.ContainsAny(stamp[15:], "\r\n\x00")) {
			return 0, 0, false
		}
		stamp = stamp[:14]
		when, err := time.ParseInLocation("20060102150405", stamp, time.Local)
		return when.Unix(), 0, err == nil && when.Format("20060102150405") == stamp
	}
	m := revisionPattern.FindStringSubmatch(revision)
	if m == nil {
		return 0, 0, false
	}
	stamp, err1 := strconv.ParseInt(m[1], 10, 64)
	pid, err2 := strconv.ParseInt(m[2], 10, 64)
	return stamp, pid, err1 == nil && err2 == nil
}

// listRevisions 列出一台设备在远端实际存在的快照，新的在前。
// 按数值比先后：进程号位数不同时按字符串比是错的。
func listRevisions(store remoteStore, id string) ([]string, error) {
	entries, err := store.List("devices/" + id)
	if err != nil {
		return nil, err
	}
	var revisions []string
	for _, entry := range entries {
		revision, ok := strings.CutSuffix(entry.Name, ".json")
		if _, _, valid := revisionOrder(revision); ok && valid && !entry.Dir {
			revisions = append(revisions, revision)
		}
	}
	sort.Slice(revisions, func(i, j int) bool {
		si, pi, _ := revisionOrder(revisions[i])
		sj, pj, _ := revisionOrder(revisions[j])
		return si > sj || si == sj && pi > pj
	})
	return revisions, nil
}

// uploadSnapshot 把 file（全部设置备份）传成一份带设备信息的文件，清掉本机多余的旧备份。
func uploadSnapshot(store remoteStore, id, hostname, file string, now time.Time) (syncDevice, error) {
	return uploadVersionedSnapshot(store, id, hostname, file, now, syncVersion{}, keepSnapshots)
}

func uploadVersionedSnapshot(store remoteStore, id, hostname, file string, now time.Time, version syncVersion, keep int) (syncDevice, error) {
	if !deviceIDPattern.MatchString(id) {
		return syncDevice{}, errors.New("设备标识不对，请重新保存同步设置")
	}
	body, err := os.ReadFile(file)
	if err != nil {
		return syncDevice{}, err
	}
	// 上传前读取同层快照，保留份数仍按设备计算，不删其他电脑的备份。
	snapshots, err := listFlatSnapshots(store, true)
	if err != nil {
		return syncDevice{}, err
	}
	used := make(map[string]bool, len(snapshots))
	for _, snapshot := range snapshots {
		used[snapshot.Revision] = true
	}
	device := syncDevice{
		ID: id, Hostname: hostname, Uploaded: now.Unix(), Size: int64(len(body)), flat: true,
		Modified: version.Modified, Origin: version.Origin, Hash: version.Hash,
	}
	stamp := now
	for conflicts := 0; ; conflicts++ {
		if conflicts == 100 {
			return syncDevice{}, errors.New("远端同名备份持续冲突，请稍后重试")
		}
		for {
			device.Revision = "sync-" + stamp.Format("20060102150405") + "-" + syncComputerName(hostname, id)
			stamp = stamp.Add(time.Second)
			if !used[device.Revision] {
				break
			}
		}
		data, err := packSyncSnapshot(body, device)
		if err != nil {
			return syncDevice{}, err
		}
		err = store.PutNew(device.snapshotKey(), bytes.NewReader(data), int64(len(data)))
		if errors.Is(err, errRemoteExists) {
			continue
		}
		if err != nil {
			return syncDevice{}, err
		}
		break
	}
	// 只清理上传前列出的旧快照，刚上传的始终保留。清理失败留待下次上传再处理。
	kept := 1
	for _, snapshot := range snapshots {
		if snapshot.ID == id {
			if kept >= keep {
				_ = store.Delete(snapshot.snapshotKey())
			}
			kept++
		}
	}
	return device, nil
}

// listDevices 列出远端有快照的设备，最近上传的在前。读不懂的设备信息跳过。
func listDevices(store remoteStore) ([]syncDevice, error) {
	return readDevices(store, false)
}

// 自动覆盖前必须读懂全部设备记录，不能跳过损坏记录后错误宣布本机最新。
func readDevices(store remoteStore, strict bool) ([]syncDevice, error) {
	snapshots, err := listFlatSnapshots(store, strict)
	if err != nil {
		return nil, err
	}
	devices, err := readDevicesAt(store, strict)
	if err != nil {
		return nil, err
	}
	snapshots = append(snapshots, devices...)
	if source, ok := store.(interface{ legacyStore() remoteStore }); ok && source.legacyStore() != nil {
		older, err := readDevicesAt(source.legacyStore(), strict)
		if err != nil {
			return nil, err
		}
		for _, old := range older {
			old.legacy = true
			snapshots = append(snapshots, old)
		}
	}
	latest := map[string]syncDevice{}
	for _, candidate := range snapshots {
		current, exists := latest[candidate.ID]
		comparison := compareSyncVersions(deviceVersion(candidate), deviceVersion(current))
		if !exists || comparison > 0 || (comparison == 0 && (candidate.Uploaded > current.Uploaded || candidate.Uploaded == current.Uploaded && candidate.Revision > current.Revision)) {
			latest[candidate.ID] = candidate
		}
	}
	devices = nil
	for _, device := range latest {
		devices = append(devices, device)
	}
	sort.Slice(devices, func(i, j int) bool { return devices[i].Uploaded > devices[j].Uploaded })
	return devices, nil
}

// 新目录与旧目录独立读取，连接或校验失败不能当作「云端没有数据」。
func readDevicesAt(store remoteStore, strict bool) ([]syncDevice, error) {
	entries, err := store.List("devices")
	if err != nil {
		return nil, err
	}
	var devices []syncDevice
	for _, entry := range entries {
		id, ok := strings.CutSuffix(entry.Name, ".json")
		if !ok || entry.Dir || !deviceIDPattern.MatchString(id) {
			continue
		}
		var data bytes.Buffer
		if err := store.Get(deviceInfoKey(id), &data); err != nil {
			return nil, err
		}
		var device syncDevice
		if json.Unmarshal(data.Bytes(), &device) != nil {
			if strict {
				return nil, errors.New("云端设备信息损坏，自动同步已停止")
			}
			continue
		}
		// 标识以文件名为准，不信文件里写的（拿它拼路径）
		device.ID = id
		if _, _, valid := revisionOrder(device.Revision); !valid {
			if strict {
				return nil, errors.New("云端备份编号无效，自动同步已停止")
			}
			device.Revision = ""
		}
		if strict && (device.Modified < 0 || device.Uploaded <= 0 || (device.Hash != "" && (len(device.Hash) != 64 || device.Modified <= 0 || !deviceIDPattern.MatchString(device.Origin)))) {
			return nil, errors.New("云端备份版本信息无效，自动同步已停止")
		}
		devices = append(devices, device)
	}
	sort.Slice(devices, func(i, j int) bool { return devices[i].Uploaded > devices[j].Uploaded })
	return devices, nil
}

// downloadSnapshot 把一台设备最近的快照下载到 file。设备信息指向的快照不在了，
// 就退回这台设备目录下实际存在的最新一份。
func downloadSnapshot(store remoteStore, device syncDevice, file string) error {
	if device.flat {
		first := downloadExactSnapshot(store, device, file)
		if first == nil {
			return nil
		}
		snapshots, err := listFlatSnapshots(store, false)
		if err != nil {
			return err
		}
		for _, snapshot := range snapshots {
			if snapshot.ID == device.ID && snapshot.Revision != device.Revision {
				return downloadExactSnapshot(store, snapshot, file)
			}
		}
		return first
	}
	store = device.snapshotStore(store)
	if !deviceIDPattern.MatchString(device.ID) {
		return errors.New("设备标识不对")
	}
	fetch := func(revision string) error {
		out, err := os.Create(file)
		if err != nil {
			return err
		}
		err = store.Get(snapshotKey(device.ID, revision), out)
		if closeErr := out.Close(); err == nil {
			err = closeErr
		}
		return err
	}
	var first error
	if device.Revision != "" {
		if first = fetch(device.Revision); first == nil {
			return nil
		}
	}
	revisions, err := listRevisions(store, device.ID)
	if err != nil {
		return err
	}
	for _, revision := range revisions {
		if revision != device.Revision {
			return fetch(revision)
		}
	}
	if first != nil {
		return first
	}
	return errors.New("这台电脑在云端没有备份")
}

// ---------- 设置页里的动作 ----------

func (s *settings) syncDataDir() string {
	if s.syncDir != "" {
		return s.syncDir
	}
	return s.status.DataDir
}

// loadSync 第一次打开「数据与同步」页时读一次 sync.json。
func (s *settings) loadSync() {
	dir := s.syncDataDir()
	if s.syncLoaded || dir == "" {
		return
	}
	s.syncLoaded = true
	c, err := loadSyncSettings(dir)
	s.sync = c
	if err != nil {
		s.syncMessage, s.syncFailed = err.Error(), true
	}
}

func (s *settings) saveSync() bool {
	if err := validateSyncPlan(s.sync); err != nil && s.sync.Enabled && s.sync.Automatic {
		s.syncMessage, s.syncFailed = err.Error(), true
		return false
	}
	if err := saveSyncSettings(s.syncDataDir(), s.sync); err != nil {
		s.syncMessage, s.syncFailed = err.Error(), true
		return false
	}
	if s.autoSync != nil {
		s.autoSync.configure(s.sync)
	}
	return true
}

// syncTask 在后台做一件要连远端的事；同一时间只做一件，做完显示结果。
func (s *settings) syncTask(area, doing string, work func(store remoteStore) (string, func(), error)) {
	if s.syncBusy != "" {
		return
	}
	s.syncMessageArea = area
	if !s.saveSync() {
		return
	}
	manager := s.autoSync
	if manager != nil && !manager.gate.TryLock() {
		s.syncMessage, s.syncFailed = "自动同步正在进行，请稍后重试", true
		return
	}
	settings := s.sync
	s.syncBusy, s.syncMessage, s.syncFailed = doing, "", false
	s.background(func() func() {
		if manager != nil {
			defer manager.gate.Unlock()
		}
		var (
			message string
			change  func()
		)
		store, err := s.openStore(settings)
		if err == nil && settings.Encrypt && settings.EncryptPassword == "" {
			err = errNoPassword
		}
		if err == nil {
			message, change, err = work(withBackupPassword(store, settings))
		}
		return func() {
			s.syncBusy = ""
			if err != nil {
				s.syncMessage, s.syncFailed = err.Error(), true
				return
			}
			s.syncMessage = message
			if change != nil {
				change()
			}
		}
	})
}

func (s *settings) openStore(c syncSettings) (remoteStore, error) {
	if s.newStore != nil {
		return s.newStore(c)
	}
	return openRemoteStore(c)
}

func (s *settings) testSync() {
	s.syncTask("connection", "正在测试连接", func(store remoteStore) (string, func(), error) {
		return "连接成功，可以上传和恢复", nil, store.Probe()
	})
}

// tempSnapshot 给快照找一个临时文件名（服务进程写、设置程序读，或反过来），用完删掉。
func tempSnapshot() (string, error) {
	file, err := os.CreateTemp("", "freetype-sync-*.json")
	if err != nil {
		return "", err
	}
	name := file.Name()
	return name, file.Close()
}

func (s *settings) uploadSync() {
	id := s.sync.DeviceID
	config := s.sync
	manager := s.autoSync
	s.syncTask("transfer", "正在上传", func(store remoteStore) (string, func(), error) {
		file, err := tempSnapshot()
		if err != nil {
			return "", nil, err
		}
		defer os.Remove(file)
		// 快照就是「导出全部设置」的文件：由服务进程生成，里面没有同步的地址和凭据
		operation := "export_settings"
		if manager != nil {
			operation = "export_sync_settings"
		}
		response, err := s.backend.Call(op(operation, "path", file))
		if err != nil {
			return "", nil, fmt.Errorf("生成备份失败：%w", err)
		}
		var version syncVersion
		if manager != nil {
			hash, err := snapshotHash(file)
			if err != nil {
				return "", nil, err
			}
			manager.mu.Lock()
			version = localSyncVersion(&manager.state.Checkpoint, syncTarget(config), hash, response.ModifiedUnixMS, id)
			manager.state.Checkpoint.Version = version
			err = manager.saveLocked()
			manager.mu.Unlock()
			if err != nil {
				return "", nil, err
			}
		}
		hostname, _ := os.Hostname()
		device, err := uploadVersionedSnapshot(store, id, hostname, file, time.Now(), version, config.KeepSnapshots)
		if err != nil {
			return "", nil, err
		}
		devices, _ := listDevices(store)
		return fmt.Sprintf("已上传；%s", sizeText(device.Size)), func() {
			if devices != nil {
				s.syncDevices, s.syncListed = devices, true
			}
		}, nil
	})
}

func (s *settings) refreshSyncDevices() {
	s.syncTask("transfer", "正在读取设备", func(store remoteStore) (string, func(), error) {
		devices, err := listDevices(store)
		if err != nil {
			return "", nil, err
		}
		message := fmt.Sprintf("云端有 %d 台电脑的备份", len(devices))
		if len(devices) == 0 {
			message = "云端还没有备份；请先在一台电脑上点击立即上传"
		}
		return message, func() { s.syncDevices, s.syncListed = devices, true }, nil
	})
}

func (s *settings) restoreSync(device syncDevice) {
	s.syncTask("transfer", "正在恢复", func(store remoteStore) (string, func(), error) {
		file, err := tempSnapshot()
		if err != nil {
			return "", nil, err
		}
		defer os.Remove(file)
		if err := downloadSnapshot(store, device, file); err != nil {
			return "", nil, err
		}
		response, err := s.backend.Call(op("merge_settings", "path", file))
		if err != nil {
			return "", nil, fmt.Errorf("合并失败：%w", err)
		}
		message := "已合并 " + deviceName(device) + " 的备份"
		if response.Report != nil && response.Report.Note != "" {
			message += "：" + response.Report.Note
		}
		return message, func() { s.applyRestored(response) }, nil
	})
}

// applyRestored 把导入、合并后的数据换到界面上（两种恢复共用）。
func (s *settings) applyRestored(response Response) {
	if response.Config != nil {
		s.config = *response.Config
		s.loaded = true
	}
	if response.Status != nil {
		s.status = *response.Status
		s.statusErr = ""
	}
	s.dicts, s.phrases, s.common = response.Dictionaries, response.QuickPhrases, response.CommonPhrases
	s.words, s.wordsLoaded = response.UserWords, true
	s.report, s.reportErrors = "", nil
	s.updateBrandIcon()
	if s.onTray != nil {
		s.onTray(s.config.TrayIcon)
	}
	if s.onAutomaticUpdate != nil {
		s.onAutomaticUpdate(s.config.AutoUpdate)
	}
}

func deviceName(device syncDevice) string {
	if name := strings.TrimSpace(device.Hostname); name != "" {
		return name
	}
	// 没报电脑名的设备用标识的前几位称呼
	return device.ID[:min(8, len(device.ID))]
}

func sizeText(size int64) string {
	switch {
	case size >= 1<<20:
		return fmt.Sprintf("%.1f MB", float64(size)/(1<<20))
	case size >= 1<<10:
		return fmt.Sprintf("%.1f KB", float64(size)/(1<<10))
	}
	return fmt.Sprintf("%d 字节", size)
}
