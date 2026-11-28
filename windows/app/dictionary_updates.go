package main

import (
	"archive/zip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/google/go-github/v93/github"
)

// 词库更新包由仓库每周的工作流发布为 freetype-data-<版本> Release（不占用程序的 Latest）。
// 设置程序负责检查、下载、校验和解包；服务进程只登记待生效版本，并在下次启动时切换。
const (
	dataTagPrefix     = "freetype-data-"
	dataChannel       = "freetype-data"
	dataPackageFormat = 1
	dataManifestName  = "PACKAGE.json"
	// 自动更新每七天检查一次；失败后隔六小时再试，不在启动循环中反复请求。
	dataCheckInterval = 7 * 24 * time.Hour
	dataRetryInterval = 6 * time.Hour
	// 许可、来源说明等附带文件的总上限；词库文件按清单记录的大小限制。
	dataExtraLimit = 16 << 20
)

// 需与服务进程 dictionary_updates::FILES 和 LEXICON_FORMATS 一致。
var (
	dataFiles          = []string{"english.txt", "lexicon.ftd"}
	dataLexiconFormats = map[string]bool{"FTD1": true, "FTD2": true}
	dataExtraFiles     = map[string]bool{"LICENSE": true, "SOURCE.txt": true, "WANXIANG-LICENSE-SCOPE.md": true, "WANXIANG-NOTICE.md": true}
)

// DictionaryUpdateStatus 与服务进程 freetype_protocol::DictionaryUpdateStatus 一致。
type DictionaryUpdateStatus struct {
	Directory        string `json:"directory"`
	Active           string `json:"active"`
	Bundled          string `json:"bundled"`
	Current          string `json:"current"`
	Previous         string `json:"previous"`
	Pending          string `json:"pending"`
	PendingAutomatic bool   `json:"pending_automatic"`
	FailedVersion    string `json:"failed_version"`
	FailedReason     string `json:"failed_reason"`
}

type dataFile struct {
	Bytes  int64  `json:"bytes"`
	SHA256 string `json:"sha256"`
}

type dataPackage struct {
	Format        int                 `json:"format"`
	Channel       string              `json:"channel"`
	Version       string              `json:"version"`
	LexiconFormat string              `json:"lexiconFormat"`
	Files         map[string]dataFile `json:"files"`
	// 只出现在 Release 的独立清单里，记录压缩包本身。
	Archive *struct {
		Name   string `json:"name"`
		Bytes  int64  `json:"bytes"`
		SHA256 string `json:"sha256"`
	} `json:"archive,omitempty"`
}

type dataCandidate struct {
	version  string
	manifest dataPackage
	zipID    int64
	digest   string
}

type dataUpdateState struct {
	Status    DictionaryUpdateStatus
	Available string
	Message   string
	Busy      bool
	Loaded    bool
}

type dataUpdates struct {
	api        *github.Client
	http       *http.Client
	backend    Backend
	owner      string
	repository string
	now        func() time.Time

	mu         sync.Mutex
	state      dataUpdateState
	candidate  *dataCandidate
	automatic  bool
	autoCancel context.CancelFunc
	timer      *time.Timer
	generation uint64
	changed    func(dataUpdateState)
	operation  sync.Mutex
}

func newDataUpdates(backend Backend, client *http.Client, base string) (*dataUpdates, error) {
	options := []github.ClientOptionsFunc{github.WithHTTPClient(client)}
	if base != "" { // 本地测试服务；正式环境使用 GitHub 默认地址。
		options = append(options, github.WithURLs(&base, &base))
	}
	api, err := github.NewClient(options...)
	if err != nil {
		return nil, err
	}
	return &dataUpdates{api: api, http: client, backend: backend, owner: "OUBIGFA", repository: "FreeType",
		now: time.Now, state: dataUpdateState{Message: "点击“检查词库更新”查看可用版本"}}, nil
}

func (m *dataUpdates) snapshot() dataUpdateState {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.state
}

func (m *dataUpdates) change(fn func(*dataUpdateState)) {
	m.mu.Lock()
	fn(&m.state)
	state, changed := m.state, m.changed
	m.mu.Unlock()
	if changed != nil {
		changed(state)
	}
}

// call 发送词库更新请求并记下服务返回的最新状态。
func (m *dataUpdates) call(request Request) (DictionaryUpdateStatus, error) {
	response, err := m.backend.Call(request)
	if response.DictionaryUpdate != nil {
		status := *response.DictionaryUpdate
		m.change(func(s *dataUpdateState) { s.Status, s.Loaded = status, true })
		return status, err
	}
	if err == nil {
		err = errors.New("服务未返回词库更新状态")
	}
	return DictionaryUpdateStatus{}, err
}

func (m *dataUpdates) refresh() error {
	_, err := m.call(op("dictionary_update_status"))
	return err
}

// dataVersion 把 2026.11.29.1 与安装包的 2026-11-14.1 换成可比较的数字。
func dataVersion(text string) ([4]int, bool) {
	var out [4]int
	parts := strings.FieldsFunc(text, func(r rune) bool { return r == '.' || r == '-' })
	if len(parts) != 4 {
		return out, false
	}
	for i, part := range parts {
		n, err := strconv.Atoi(part)
		if err != nil || n < 0 || part == "" || strings.TrimLeft(part, "0123456789") != "" {
			return out, false
		}
		out[i] = n
	}
	return out, true
}

func newerData(a, b string) bool {
	x, okA := dataVersion(a)
	y, okB := dataVersion(b)
	if !okA {
		return false
	}
	if !okB {
		return true
	}
	for i := range x {
		if x[i] != y[i] {
			return x[i] > y[i]
		}
	}
	return false
}

// baseline 是已经在用或已安排生效的最新版本；更旧或相同的版本不再下载。
func (status DictionaryUpdateStatus) baseline() string {
	best := status.Bundled
	for _, v := range []string{status.Active, status.Current, status.Pending} {
		if newerData(v, best) {
			best = v
		}
	}
	return best
}

func (p dataPackage) compatible(version string) error {
	if p.Format != dataPackageFormat || p.Channel != dataChannel || p.Version != version {
		return errors.New("更新包格式不受这个版本的 FreeType 支持")
	}
	if !dataLexiconFormats[p.LexiconFormat] {
		return fmt.Errorf("更新包的词库格式 %s 需要更新 FreeType", p.LexiconFormat)
	}
	if len(p.Files) != len(dataFiles) {
		return errors.New("更新包的文件范围与本程序不一致")
	}
	for _, name := range dataFiles {
		if f, ok := p.Files[name]; !ok || f.Bytes <= 0 || len(f.SHA256) != 64 {
			return errors.New("更新包的文件范围与本程序不一致")
		}
	}
	return nil
}

// latest 找出比 baseline 新、且本程序能读的最新词库版本；格式不兼容的新版本跳过，不凭日期加载。
// newerFormat 表示还有更新的词库，但需要先更新 FreeType 才能使用。
func (m *dataUpdates) latest(ctx context.Context, status DictionaryUpdateStatus, automatic bool) (candidate *dataCandidate, newerFormat bool, err error) {
	releases, _, err := m.api.Repositories.ListReleases(ctx, m.owner, m.repository, &github.ListOptions{PerPage: 50})
	if err != nil {
		return nil, false, err
	}
	sort.SliceStable(releases, func(i, j int) bool {
		return newerData(strings.TrimPrefix(releases[i].GetTagName(), dataTagPrefix), strings.TrimPrefix(releases[j].GetTagName(), dataTagPrefix))
	})
	var lastErr error
	for _, release := range releases {
		tag := release.GetTagName()
		version := strings.TrimPrefix(tag, dataTagPrefix)
		if release.GetDraft() || release.GetPrerelease() || !strings.HasPrefix(tag, dataTagPrefix) || !strings.Contains(version, ".") || strings.Contains(version, "-") {
			continue
		}
		if !newerData(version, status.baseline()) {
			break
		}
		if automatic && version == status.FailedVersion {
			continue
		}
		var manifestAsset, zipAsset *github.ReleaseAsset
		for _, asset := range release.Assets {
			switch asset.GetName() {
			case "FreeType-Dictionaries-" + version + ".json":
				manifestAsset = asset
			case "FreeType-Dictionaries-" + version + ".zip":
				zipAsset = asset
			}
		}
		if manifestAsset == nil || zipAsset == nil || manifestAsset.GetSize() > 1<<20 {
			continue
		}
		manifest, err := m.readManifest(ctx, manifestAsset.GetID())
		if err != nil {
			lastErr = err
			continue
		}
		if manifest.compatible(version) != nil {
			newerFormat = true
			continue
		}
		if manifest.Archive == nil || manifest.Archive.Name != zipAsset.GetName() || manifest.Archive.Bytes != int64(zipAsset.GetSize()) || len(manifest.Archive.SHA256) != 64 {
			lastErr = errors.New("更新包清单与附件不一致")
			continue
		}
		return &dataCandidate{version: version, manifest: manifest, zipID: zipAsset.GetID(),
			digest: strings.TrimPrefix(zipAsset.GetDigest(), "sha256:")}, newerFormat, nil
	}
	return nil, newerFormat, lastErr
}

func (m *dataUpdates) readManifest(ctx context.Context, id int64) (dataPackage, error) {
	var manifest dataPackage
	reader, _, err := m.api.Repositories.DownloadReleaseAsset(ctx, m.owner, m.repository, id, m.http)
	if err != nil {
		return manifest, err
	}
	defer reader.Close()
	data, err := io.ReadAll(io.LimitReader(reader, 1<<20))
	if err != nil {
		return manifest, err
	}
	return manifest, json.Unmarshal(data, &manifest)
}

// install 下载、校验并解包到 <目录>/<版本>，全部通过后才请服务进程登记为下次启动生效。
// 任何一步失败都只删除本次的暂存目录，不改变正在使用的词库。
func (m *dataUpdates) install(ctx context.Context, status DictionaryUpdateStatus, candidate *dataCandidate, automatic bool) error {
	if status.Directory == "" {
		return errors.New("服务未提供词库更新目录")
	}
	target := filepath.Join(status.Directory, candidate.version)
	partial := target + ".partial"
	if err := os.RemoveAll(partial); err != nil {
		return err
	}
	if err := os.MkdirAll(partial, 0o700); err != nil {
		return err
	}
	defer os.RemoveAll(partial)
	archive := filepath.Join(partial, "package.zip")
	if err := m.download(ctx, candidate, archive); err != nil {
		return err
	}
	if err := extractDataPackage(archive, partial, candidate.manifest); err != nil {
		return err
	}
	if err := os.Remove(archive); err != nil {
		return err
	}
	if status.Current != candidate.version && status.Previous != candidate.version {
		if err := os.RemoveAll(target); err != nil {
			return err
		}
		if err := os.Rename(partial, target); err != nil {
			return err
		}
	}
	_, err := m.call(op("stage_dictionary_update", "version", candidate.version, "automatic", automatic))
	return err
}

func (m *dataUpdates) download(ctx context.Context, candidate *dataCandidate, path string) error {
	reader, _, err := m.api.Repositories.DownloadReleaseAsset(ctx, m.owner, m.repository, candidate.zipID, m.http)
	if err != nil {
		return err
	}
	defer reader.Close()
	file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	sum := sha256.New()
	expected := candidate.manifest.Archive
	n, err := io.Copy(io.MultiWriter(file, sum), io.LimitReader(reader, expected.Bytes+1))
	if closeErr := file.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	digest := hex.EncodeToString(sum.Sum(nil))
	if n != expected.Bytes {
		return fmt.Errorf("词库更新包大小不正确：收到 %d 字节", n)
	}
	// 清单摘要与 GitHub 记录的附件摘要都要一致（GitHub 未提供时只核对清单）。
	if digest != expected.SHA256 || (candidate.digest != "" && digest != candidate.digest) {
		return errors.New("词库更新包校验失败，请稍后重试")
	}
	return nil
}

// extractDataPackage 只接受清单列出的词库文件和固定的许可说明，文件名不得含路径，
// 每个文件按清单的大小和摘要核对；包内清单必须与 Release 清单描述同一组文件。
func extractDataPackage(archive, dir string, manifest dataPackage) error {
	reader, err := zip.OpenReader(archive)
	if err != nil {
		return fmt.Errorf("词库更新包无法打开：%w", err)
	}
	defer reader.Close()
	seen := map[string]bool{}
	var extra int64
	for _, entry := range reader.File {
		name := entry.Name
		expected, listed := manifest.Files[name]
		if strings.ContainsAny(name, `/\:`) || seen[name] || (!listed && name != dataManifestName && !dataExtraFiles[name]) {
			return fmt.Errorf("词库更新包包含意外的文件：%s", name)
		}
		seen[name] = true
		limit := expected.Bytes
		if !listed {
			limit = dataExtraLimit - extra
		}
		written, digest, err := extractEntry(entry, filepath.Join(dir, name), limit)
		if err != nil {
			return err
		}
		if listed && (written != expected.Bytes || digest != expected.SHA256) {
			return fmt.Errorf("词库更新包的 %s 校验失败", name)
		}
		if !listed {
			extra += written
			if extra > dataExtraLimit {
				return errors.New("词库更新包的附带文件过大")
			}
		}
	}
	for _, name := range append(dataFiles, dataManifestName) {
		if !seen[name] {
			return fmt.Errorf("词库更新包缺少 %s", name)
		}
	}
	data, err := os.ReadFile(filepath.Join(dir, dataManifestName))
	if err != nil {
		return err
	}
	var inner dataPackage
	if err := json.Unmarshal(data, &inner); err != nil {
		return fmt.Errorf("词库更新包的清单无法读取：%w", err)
	}
	if err := inner.compatible(manifest.Version); err != nil {
		return err
	}
	for name, file := range manifest.Files {
		if inner.Files[name] != file {
			return errors.New("词库更新包的清单与发布清单不一致")
		}
	}
	return nil
}

func extractEntry(entry *zip.File, path string, limit int64) (int64, string, error) {
	source, err := entry.Open()
	if err != nil {
		return 0, "", err
	}
	defer source.Close()
	target, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return 0, "", err
	}
	sum := sha256.New()
	n, err := io.Copy(io.MultiWriter(target, sum), io.LimitReader(source, limit+1))
	if closeErr := target.Close(); err == nil {
		err = closeErr
	}
	if err == nil && n > limit {
		err = fmt.Errorf("词库更新包的 %s 超过清单记录的大小", entry.Name)
	}
	return n, hex.EncodeToString(sum.Sum(nil)), err
}

func (m *dataUpdates) begin(message string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.state.Busy {
		return false
	}
	m.state.Busy, m.state.Message = true, message
	return true
}

func (m *dataUpdates) finish(fn func(*dataUpdateState)) {
	m.change(func(s *dataUpdateState) {
		s.Busy = false
		fn(s)
	})
}

// check 只查看是否有新版本；手动模式由用户再点“下载更新”。
func (m *dataUpdates) check(ctx context.Context) error {
	m.operation.Lock()
	defer m.operation.Unlock()
	if !m.begin("正在检查词库更新") {
		return nil
	}
	status, err := m.call(op("dictionary_update_status"))
	var candidate *dataCandidate
	newerFormat := false
	if err == nil {
		candidate, newerFormat, err = m.latest(ctx, status, false)
	}
	m.mu.Lock()
	m.candidate = candidate
	m.mu.Unlock()
	m.finish(func(s *dataUpdateState) {
		switch {
		case err != nil:
			s.Message, s.Available = "检查词库更新失败："+err.Error(), ""
		case candidate == nil && newerFormat:
			s.Message, s.Available = "有更新的词库，需要先更新 FreeType", ""
		case candidate == nil:
			s.Message, s.Available = "词库已是最新", ""
		default:
			s.Message, s.Available = "发现词库 "+candidate.version+"；点击“下载更新”", candidate.version
		}
	})
	return err
}

// downloadAvailable 下载最近一次检查到的版本；自动更新开关保持原状。
func (m *dataUpdates) downloadAvailable(ctx context.Context) error {
	m.operation.Lock()
	defer m.operation.Unlock()
	m.mu.Lock()
	candidate := m.candidate
	m.mu.Unlock()
	if candidate == nil || !m.begin("正在下载词库 "+candidate.version) {
		return nil
	}
	status, err := m.call(op("dictionary_update_status"))
	if err == nil {
		err = m.install(ctx, status, candidate, false)
	}
	m.finish(func(s *dataUpdateState) {
		if err != nil {
			s.Message = "下载词库更新失败：" + err.Error()
			return
		}
		s.Available = ""
		s.Message = "词库 " + candidate.version + " 已下载；下次服务启动时生效"
	})
	return err
}

// runAutomatic 检查并下载最新的兼容版本；记录成功检查的时间，未到七天不再检查。
func (m *dataUpdates) runAutomatic(ctx context.Context) error {
	m.operation.Lock()
	defer m.operation.Unlock()
	status, err := m.call(op("dictionary_update_status"))
	if err != nil {
		return err
	}
	if last, ok := readDataChecked(status.Directory); ok && m.now().Sub(last) < dataCheckInterval {
		return nil
	}
	if !m.begin("正在自动检查词库更新") {
		return nil
	}
	candidate, _, err := m.latest(ctx, status, true)
	if err == nil && candidate != nil {
		m.change(func(s *dataUpdateState) { s.Message = "正在下载词库 " + candidate.version })
		err = m.install(ctx, status, candidate, true)
	}
	if err == nil {
		err = writeDataChecked(status.Directory, m.now())
	}
	m.finish(func(s *dataUpdateState) {
		switch {
		case errors.Is(err, context.Canceled):
			s.Message = "已停止自动更新词库"
		case err != nil:
			s.Message = "自动更新词库失败：" + err.Error() + "；稍后重试"
		case candidate == nil:
			s.Message = "词库已是最新"
		default:
			s.Available = ""
			s.Message = "词库 " + candidate.version + " 已下载；下次服务启动时生效"
		}
	})
	return err
}

func readDataChecked(dir string) (time.Time, bool) {
	var record struct {
		LastSuccess time.Time `json:"lastSuccess"`
	}
	data, err := os.ReadFile(filepath.Join(dir, "checked.json"))
	if err != nil || json.Unmarshal(data, &record) != nil || record.LastSuccess.IsZero() {
		return time.Time{}, false
	}
	return record.LastSuccess, true
}

func writeDataChecked(dir string, at time.Time) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	data, _ := json.Marshal(map[string]time.Time{"lastSuccess": at.UTC()})
	temp := filepath.Join(dir, "checked.json.tmp")
	if err := os.WriteFile(temp, data, 0o600); err != nil {
		return err
	}
	return os.Rename(temp, filepath.Join(dir, "checked.json"))
}

// setAutomatic 开启时在后台检查一次，之后按七天间隔；关闭时取消进行中的自动下载，
// 并撤销自动安排但尚未生效的版本（已生效的版本不回退）。
func (m *dataUpdates) setAutomatic(on bool) {
	m.mu.Lock()
	if m.automatic == on {
		m.mu.Unlock()
		return
	}
	m.automatic = on
	m.generation++
	if m.autoCancel != nil {
		m.autoCancel()
		m.autoCancel = nil
	}
	if m.timer != nil {
		m.timer.Stop()
		m.timer = nil
	}
	m.mu.Unlock()
	if on {
		m.schedule(0)
		return
	}
	go func() { _, _ = m.call(op("cancel_pending_dictionary", "automatic_only", true)) }()
}

func (m *dataUpdates) schedule(delay time.Duration) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if !m.automatic {
		return
	}
	generation := m.generation
	m.timer = time.AfterFunc(delay, func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Minute)
		m.mu.Lock()
		if generation != m.generation {
			m.mu.Unlock()
			cancel()
			return
		}
		m.autoCancel = cancel
		m.mu.Unlock()
		err := m.runAutomatic(ctx)
		cancel()
		next := dataRetryInterval
		if err == nil {
			// 重新读取记录的时间：每小时看一次是否到了七天，检查本身仍是七天一次。
			next = time.Hour
		}
		m.mu.Lock()
		current := generation == m.generation
		m.mu.Unlock()
		if current {
			m.schedule(next)
		}
	})
}

func (m *dataUpdates) restorePrevious() error {
	_, err := m.call(op("restore_previous_dictionary"))
	m.change(func(s *dataUpdateState) {
		if err != nil {
			s.Message = "恢复上一版失败：" + err.Error()
		} else {
			s.Message = "已安排恢复上一版；下次服务启动时生效"
		}
	})
	return err
}

func (m *dataUpdates) cancelPending() error {
	_, err := m.call(op("cancel_pending_dictionary", "automatic_only", false))
	m.change(func(s *dataUpdateState) {
		if err != nil {
			s.Message = "取消失败：" + err.Error()
		} else {
			s.Message = "已取消待生效的词库"
		}
	})
	return err
}

func (m *dataUpdates) close() {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.generation++
	if m.autoCancel != nil {
		m.autoCancel()
	}
	if m.timer != nil {
		m.timer.Stop()
	}
}
