package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"unicode"
)

// 电脑名用于辨认；Windows 文件名不接受的字符替换为下划线。
func syncComputerName(hostname, id string) string {
	name := strings.Trim(strings.Map(func(r rune) rune {
		if unicode.IsControl(r) || strings.ContainsRune(`<>:"/\|?*`, r) {
			return '_'
		}
		return r
	}, hostname), " .")
	runes := []rune(name)
	if len(runes) > 80 {
		name = strings.TrimRight(string(runes[:80]), " .")
	}
	if name == "" {
		name = "电脑-" + id[:8]
	}
	return name
}

// 首行保存小型索引，其余字节保持核心导出的原样；恢复时移除首行即可还原原始摘要。
// 文件仍是完整设置 JSON，可直接通过「一键导入」恢复。
func packSyncSnapshot(body []byte, device syncDevice) ([]byte, error) {
	var fields map[string]json.RawMessage
	if len(body) == 0 || body[0] != '{' || json.Unmarshal(body, &fields) != nil || len(fields) == 0 || fields["sync_metadata"] != nil {
		return nil, errors.New("生成的同步备份格式不正确")
	}
	meta, err := json.Marshal(device)
	if err != nil {
		return nil, err
	}
	data := append([]byte(`{"sync_metadata":`), meta...)
	data = append(data, ',', '\n')
	return append(data, body[1:]...), nil
}

func parseSyncHeader(line []byte, revision string) (syncDevice, error) {
	var header struct {
		Device syncDevice `json:"sync_metadata"`
	}
	line = bytes.TrimSuffix(line, []byte("\n"))
	if !bytes.HasSuffix(line, []byte(",")) {
		return syncDevice{}, errors.New("云端备份缺少同步信息")
	}
	data := append(append([]byte{}, line[:len(line)-1]...), '}')
	if json.Unmarshal(data, &header) != nil {
		return syncDevice{}, errors.New("云端备份的同步信息损坏")
	}
	d := header.Device
	_, _, valid := revisionOrder(revision)
	if !valid || !strings.HasPrefix(revision, "sync-") || d.Revision != revision || !deviceIDPattern.MatchString(d.ID) || d.Uploaded <= 0 || d.Size <= 0 || d.Modified < 0 {
		return syncDevice{}, errors.New("云端备份的设备或版本信息无效")
	}
	if d.Hash != "" {
		decoded, err := hex.DecodeString(d.Hash)
		if err != nil || len(decoded) != sha256.Size || d.Modified <= 0 || !deviceIDPattern.MatchString(d.Origin) {
			return syncDevice{}, errors.New("云端备份的校验信息无效")
		}
	}
	d.flat = true
	return d, nil
}

var errSyncHeaderComplete = errors.New("同步信息读取完成")

// 列表只读首行即关闭响应，避免每次同步都重新下载全部历史备份。
type syncHeaderReader struct{ line []byte }

func (h *syncHeaderReader) Write(p []byte) (int, error) {
	n := len(p)
	end := bytes.IndexByte(p, '\n')
	if end >= 0 {
		n = end + 1
	}
	if len(h.line)+n > 16<<10 {
		return 0, errors.New("云端备份的同步信息过长")
	}
	h.line = append(h.line, p[:n]...)
	if end >= 0 {
		return n, errSyncHeaderComplete
	}
	return n, nil
}

func listFlatSnapshots(store remoteStore, strict bool) ([]syncDevice, error) {
	entries, err := store.List("")
	if err != nil {
		return nil, err
	}
	var snapshots []syncDevice
	for _, entry := range entries {
		revision, ok := strings.CutSuffix(entry.Name, ".json")
		if !ok || entry.Dir || !strings.HasPrefix(revision, "sync-") {
			continue
		}
		reader := &syncHeaderReader{}
		err := store.Get(entry.Name, reader)
		if err != nil && !errors.Is(err, errSyncHeaderComplete) {
			return nil, err
		}
		device, err := parseSyncHeader(reader.line, revision)
		if err != nil {
			if strict {
				return nil, fmt.Errorf("%s：%w", entry.Name, err)
			}
			continue
		}
		snapshots = append(snapshots, device)
	}
	sort.Slice(snapshots, func(i, j int) bool {
		if snapshots[i].Uploaded != snapshots[j].Uploaded {
			return snapshots[i].Uploaded > snapshots[j].Uploaded
		}
		return snapshots[i].Revision > snapshots[j].Revision
	})
	return snapshots, nil
}

// 精确读取比较过的那一份；平铺快照先移除内嵌索引，再交给核心校验与导入。
func downloadExactSnapshot(store remoteStore, device syncDevice, file string) error {
	var data bytes.Buffer
	if err := device.snapshotStore(store).Get(device.snapshotKey(), &data); err != nil {
		return err
	}
	body := data.Bytes()
	if device.flat {
		end := bytes.IndexByte(body, '\n')
		if end < 0 {
			return errors.New("云端备份内容不完整")
		}
		actual, err := parseSyncHeader(body[:end+1], device.Revision)
		if err != nil {
			return err
		}
		if actual.ID != device.ID || actual.Hash != device.Hash || actual.Modified != device.Modified || actual.Origin != device.Origin || actual.Size != device.Size {
			return errors.New("云端备份已变化，请重新同步")
		}
		body = append([]byte{'{'}, body[end+1:]...)
		if int64(len(body)) != actual.Size || !json.Valid(body) {
			return errors.New("云端备份内容损坏")
		}
		if actual.Hash != "" {
			hash := sha256.Sum256(body)
			if hex.EncodeToString(hash[:]) != actual.Hash {
				return errors.New("云端备份校验失败，本机未改变")
			}
		}
	}
	return os.WriteFile(file, body, 0600)
}
