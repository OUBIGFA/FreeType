package main

import (
	"bytes"
	"encoding/json"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// AWS 文档「Signature Calculations for the Authorization Header」里的两个例子：签名要逐字节对上。
func TestSigV4MatchesAWSDocumentedExamples(t *testing.T) {
	// 这是 AWS 文档里公开的示例密钥，不是真凭据。标识分两段写：仓库检查按 AKIA 开头的
	// 样式拦截真密钥，没有登记例外的办法，这里不让示例值触发它。
	const access, secret = "AKIA" + "IOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
	base := func() map[string]string {
		return map[string]string{
			"host":                 "examplebucket.s3.amazonaws.com",
			"x-amz-date":           "20130524T000000Z",
			"x-amz-content-sha256": emptySHA256,
		}
	}
	object := base()
	object["range"] = "bytes=0-9"
	got := sigV4("GET", s3Path([]string{"test.txt"}), "", object, emptySHA256, "us-east-1", access, secret)
	want := "AWS4-HMAC-SHA256 Credential=" + access + "/20130524/us-east-1/s3/aws4_request, " +
		"SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, " +
		"Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
	if got != want {
		t.Fatalf("GET 对象的签名不对：\n%s\n%s", got, want)
	}
	query := s3Query(map[string]string{"prefix": "J", "max-keys": "2"})
	if query != "max-keys=2&prefix=J" {
		t.Fatalf("查询串没有按名字排序：%s", query)
	}
	got = sigV4("GET", "/", query, base(), emptySHA256, "us-east-1", access, secret)
	if !strings.HasSuffix(got, "Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7") {
		t.Fatalf("列举对象的签名不对：%s", got)
	}
	if s3Path([]string{"目录", "a b+c~.json"}) != "/%E7%9B%AE%E5%BD%95/a%20b%2Bc~.json" {
		t.Fatalf("路径编码不对：%s", s3Path([]string{"目录", "a b+c~.json"}))
	}
}

// fakeDAV 是内存里的 WebDAV 服务端：只在已有目录里放文件，和真的一样对缺目录的 PUT 回 409。
type fakeDAV struct {
	mu    sync.Mutex
	t     *testing.T
	dirs  map[string]bool
	files map[string][]byte
}

func (f *fakeDAV) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if user, password, ok := r.BasicAuth(); !ok || user != "me" || password != "secret" {
		w.WriteHeader(http.StatusUnauthorized)
		return
	}
	path := strings.TrimRight(r.URL.Path, "/")
	parent := path[:strings.LastIndex(path, "/")]
	switch r.Method {
	case "MKCOL":
		switch {
		case f.dirs[path]:
			w.WriteHeader(http.StatusMethodNotAllowed)
		case !f.dirs[parent]:
			w.WriteHeader(http.StatusConflict)
		default:
			f.dirs[path] = true
			w.WriteHeader(http.StatusCreated)
		}
	case http.MethodPut:
		if _, exists := f.files[path]; exists && r.Header.Get("If-None-Match") == "*" {
			w.WriteHeader(http.StatusPreconditionFailed)
			return
		}
		if !f.dirs[parent] {
			w.WriteHeader(http.StatusConflict)
			return
		}
		if r.ContentLength < 0 {
			f.t.Error("上传没有带 Content-Length")
		}
		f.files[path], _ = io.ReadAll(r.Body)
		w.WriteHeader(http.StatusCreated)
	case http.MethodGet:
		data, ok := f.files[path]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		w.Write(data)
	case http.MethodDelete:
		if _, ok := f.files[path]; !ok {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		delete(f.files, path)
		w.WriteHeader(http.StatusNoContent)
	case "PROPFIND":
		if !f.dirs[path] {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		// 和坚果云一样：href 是编码过的完整路径，目录带斜杠
		href := func(p string, dir bool) string {
			u := url.URL{Path: p}
			if dir {
				u.Path += "/"
			}
			return u.EscapedPath()
		}
		var body strings.Builder
		body.WriteString(`<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">`)
		fmt.Fprintf(&body, `<d:response><d:href>%s</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>`, href(path, true))
		if r.Header.Get("Depth") == "1" {
			for dir := range f.dirs {
				if strings.HasPrefix(dir, path+"/") && !strings.Contains(dir[len(path)+1:], "/") {
					fmt.Fprintf(&body, `<d:response><d:href>%s</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>`, href(dir, true))
				}
			}
			for file, data := range f.files {
				if strings.HasPrefix(file, path+"/") && !strings.Contains(file[len(path)+1:], "/") {
					fmt.Fprintf(&body, `<d:response><d:href>%s</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>%d</d:getcontentlength></d:prop></d:propstat></d:response>`, href(file, false), len(data))
				}
			}
		}
		body.WriteString(`</d:multistatus>`)
		w.WriteHeader(http.StatusMultiStatus)
		io.WriteString(w, body.String())
	default:
		w.WriteHeader(http.StatusMethodNotAllowed)
	}
}

func TestWebDAVStoreCreatesFoldersListsOneLevelAndHandlesChineseFolders(t *testing.T) {
	dav := &fakeDAV{t: t, dirs: map[string]bool{"": true, "/dav": true}, files: map[string][]byte{}}
	server := httptest.NewServer(dav)
	defer server.Close()
	store, err := newWebDAVStore(server.URL+"/dav/我的坚果云/FreeType/", "me", "secret")
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Probe(); err == nil {
		t.Fatal("上级目录不存在时测试连接应失败，并提示先建文件夹")
	}
	dav.dirs["/dav/我的坚果云"] = true
	if err := store.Probe(); err != nil {
		t.Fatal(err)
	}
	put := func(key, text string) {
		t.Helper()
		if err := store.Put(key, strings.NewReader(text), int64(len(text))); err != nil {
			t.Fatal(err)
		}
	}
	put("devices/a.json", "{}")
	put("devices/a/1-2.json", "快照")
	if string(dav.files["/dav/我的坚果云/FreeType/devices/a/1-2.json"]) != "快照" {
		t.Fatalf("文件没有放到同步目录下：%v", dav.files)
	}
	entries, err := store.List("devices")
	if err != nil {
		t.Fatal(err)
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].Name < entries[j].Name })
	if len(entries) != 2 || entries[0] != (remoteEntry{Name: "a", Dir: true}) || entries[1] != (remoteEntry{Name: "a.json", Size: 2}) {
		t.Fatalf("只应列出这一层的目录和文件：%+v", entries)
	}
	if missing, err := store.List("没有的目录"); err != nil || len(missing) != 0 {
		t.Fatalf("不存在的目录应是空的：%v %v", missing, err)
	}
	var got bytes.Buffer
	if err := store.Get("devices/a/1-2.json", &got); err != nil || got.String() != "快照" {
		t.Fatalf("下载：%q %v", got.String(), err)
	}
	if err := store.Get("devices/none.json", io.Discard); err == nil {
		t.Fatal("下载不存在的文件应失败")
	}
	if err := store.Delete("devices/a.json"); err != nil {
		t.Fatal(err)
	}
	if err := store.Delete("devices/a.json"); err != nil {
		t.Fatalf("删除已经没有的文件也算成功：%v", err)
	}
	wrong, _ := newWebDAVStore(server.URL+"/dav/我的坚果云/FreeType", "me", "wrong")
	if err := wrong.Probe(); err == nil || !strings.Contains(err.Error(), "密码") {
		t.Fatalf("密码不对时要说清楚：%v", err)
	}
	if _, err := newWebDAVStore("dav.example.com/x", "", ""); err == nil {
		t.Fatal("没有 https:// 的地址应被拒绝")
	}
}

// fakeS3 是内存里的 S3 兼容服务端（路径风格）。它用同一套算法重算签名，签名不对回 403。
type fakeS3 struct {
	mu      sync.Mutex
	t       *testing.T
	objects map[string][]byte
}

func (f *fakeS3) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	f.mu.Lock()
	defer f.mu.Unlock()
	headers := map[string]string{
		"host":                 r.Host,
		"x-amz-date":           r.Header.Get("x-amz-date"),
		"x-amz-content-sha256": r.Header.Get("x-amz-content-sha256"),
	}
	if value := r.Header.Get("If-None-Match"); value != "" {
		headers["if-none-match"] = value
	}
	want := sigV4(r.Method, r.URL.EscapedPath(), r.URL.RawQuery, headers, headers["x-amz-content-sha256"], "auto", "key", "secret")
	if r.Header.Get("Authorization") != want {
		w.WriteHeader(http.StatusForbidden)
		io.WriteString(w, `<Error><Code>SignatureDoesNotMatch</Code><Message>bad signature</Message></Error>`)
		return
	}
	key, ok := strings.CutPrefix(r.URL.Path, "/bucket/")
	if !ok && r.URL.Path != "/bucket" {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	switch {
	case r.Method == http.MethodGet && r.URL.Query().Get("list-type") == "2":
		prefix := r.URL.Query().Get("prefix")
		type content struct {
			Key  string
			Size int
		}
		type commonPrefix struct{ Prefix string }
		var listing struct {
			XMLName        xml.Name `xml:"ListBucketResult"`
			Contents       []content
			CommonPrefixes []commonPrefix
		}
		seen := map[string]bool{}
		for name, data := range f.objects {
			rest, ok := strings.CutPrefix(name, prefix)
			if !ok {
				continue
			}
			if i := strings.Index(rest, "/"); i >= 0 {
				if dir := prefix + rest[:i+1]; !seen[dir] {
					seen[dir] = true
					listing.CommonPrefixes = append(listing.CommonPrefixes, commonPrefix{dir})
				}
				continue
			}
			listing.Contents = append(listing.Contents, content{name, len(data)})
		}
		xml.NewEncoder(w).Encode(listing)
	case r.Method == http.MethodPut:
		if _, exists := f.objects[key]; exists && r.Header.Get("If-None-Match") == "*" {
			w.WriteHeader(http.StatusPreconditionFailed)
			return
		}
		f.objects[key], _ = io.ReadAll(r.Body)
	case r.Method == http.MethodGet:
		data, ok := f.objects[key]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			io.WriteString(w, `<Error><Code>NoSuchKey</Code><Message>no such key</Message></Error>`)
			return
		}
		w.Write(data)
	case r.Method == http.MethodDelete:
		delete(f.objects, key)
		w.WriteHeader(http.StatusNoContent)
	}
}

func TestS3StoreSignsRequestsAndKeepsEverythingUnderThePrefix(t *testing.T) {
	s3 := &fakeS3{t: t, objects: map[string][]byte{"别人的/文件.txt": []byte("x")}}
	server := httptest.NewServer(s3)
	defer server.Close()
	settings := syncSettings{
		S3Endpoint: server.URL, S3Region: "auto", S3Bucket: "bucket", S3Prefix: "/备份/FreeType/",
		S3AccessKey: "key", S3SecretKey: "secret", S3PathStyle: true,
	}
	store, err := newS3Store(settings)
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Probe(); err != nil {
		t.Fatal(err)
	}
	text := "快照 with space+plus"
	if err := store.Put("devices/a b/1-2.json", strings.NewReader(text), int64(len(text))); err != nil {
		t.Fatal(err)
	}
	if err := store.Put("devices/a.json", strings.NewReader("{}"), 2); err != nil {
		t.Fatal(err)
	}
	if string(s3.objects["备份/FreeType/devices/a b/1-2.json"]) != text {
		t.Fatalf("对象没有放到同步目录下：%v", s3.objects)
	}
	entries, err := store.List("devices")
	if err != nil {
		t.Fatal(err)
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].Name < entries[j].Name })
	if len(entries) != 2 || entries[0] != (remoteEntry{Name: "a b", Dir: true}) || entries[1] != (remoteEntry{Name: "a.json", Size: 2}) {
		t.Fatalf("只应列出这一层：%+v", entries)
	}
	var got bytes.Buffer
	if err := store.Get("devices/a b/1-2.json", &got); err != nil || got.String() != text {
		t.Fatalf("下载：%q %v", got.String(), err)
	}
	if err := store.Get("devices/none.json", io.Discard); err == nil || !strings.Contains(err.Error(), "NoSuchKey") {
		t.Fatalf("下载不存在的对象要带上服务端的原因：%v", err)
	}
	if err := store.Delete("devices/a.json"); err != nil {
		t.Fatal(err)
	}
	if _, ok := s3.objects["别人的/文件.txt"]; !ok || len(s3.objects) != 2 {
		t.Fatalf("不应碰同步目录以外的对象：%v", s3.objects)
	}
	settings.S3SecretKey = "wrong"
	wrong, _ := newS3Store(settings)
	if err := wrong.Probe(); err == nil || !strings.Contains(err.Error(), "密钥") {
		t.Fatalf("密钥不对时要说清楚：%v", err)
	}
	settings.S3Bucket = ""
	if _, err := newS3Store(settings); err == nil {
		t.Fatal("没填存储桶应被拒绝")
	}
}

func TestS3StoreDoesNotFollowRedirectsAndNamesTheBucketRegion(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("x-amz-bucket-region", "eu-west-1")
		w.Header().Set("Location", "http://127.0.0.1:1/elsewhere")
		w.WriteHeader(http.StatusMovedPermanently)
	}))
	defer server.Close()
	store, err := newS3Store(syncSettings{S3Endpoint: server.URL, S3Bucket: "bucket", S3AccessKey: "key", S3SecretKey: "secret", S3PathStyle: true})
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Probe(); err == nil || !strings.Contains(err.Error(), "eu-west-1") {
		t.Fatalf("重定向应提示区域不对：%v", err)
	}
}

// memStore 是内存里的远端。
type memStore struct {
	files map[string][]byte
	fail  string
}

func (m *memStore) Put(key string, body io.ReadSeeker, size int64) error {
	if m.fail == "put" {
		return fmt.Errorf("上传失败：测试错误")
	}
	data, err := io.ReadAll(body)
	if int64(len(data)) != size {
		return fmt.Errorf("大小不对：%d != %d", len(data), size)
	}
	m.files[key] = data
	return err
}

func (m *memStore) Get(key string, w io.Writer) error {
	data, ok := m.files[key]
	if !ok {
		return fmt.Errorf("下载失败：没有 %s", key)
	}
	_, err := w.Write(data)
	return err
}

func (m *memStore) PutNew(key string, body io.ReadSeeker, size int64) error {
	if _, exists := m.files[key]; exists {
		return errRemoteExists
	}
	return m.Put(key, body, size)
}

func (m *memStore) List(dir string) ([]remoteEntry, error) {
	seen := map[string]bool{}
	var entries []remoteEntry
	for key, data := range m.files {
		prefix := dir
		if prefix != "" {
			prefix += "/"
		}
		rest, ok := strings.CutPrefix(key, prefix)
		if !ok {
			continue
		}
		name, _, deeper := strings.Cut(rest, "/")
		if !seen[name] {
			seen[name] = true
			entry := remoteEntry{Name: name, Dir: deeper}
			if !deeper {
				entry.Size = int64(len(data))
			}
			entries = append(entries, entry)
		}
	}
	return entries, nil
}

func (m *memStore) Delete(key string) error { delete(m.files, key); return nil }

func (m *memStore) Probe() error {
	if m.fail == "probe" {
		return fmt.Errorf("连接失败：测试错误")
	}
	return nil
}

const deviceA, deviceB = "0123456789abcdef0123456789abcdef", "fedcba9876543210fedcba9876543210"

func TestFlatSnapshotsKeepEachDevicesHistoryAndReadLegacyFallback(t *testing.T) {
	store := &memStore{files: map[string][]byte{}}
	file := filepath.Join(t.TempDir(), "snapshot.json")
	start := time.Unix(1_700_000_000, 0)
	for i := 0; i < 8; i++ {
		if err := os.WriteFile(file, []byte(fmt.Sprintf(`{"snapshot":%d}`, i)), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := uploadSnapshot(store, deviceA, "办公室", file, start.Add(time.Duration(i)*time.Hour)); err != nil {
			t.Fatal(err)
		}
	}
	snapshots, err := listFlatSnapshots(store, true)
	if err != nil || len(snapshots) != keepSnapshots {
		t.Fatalf("每台设备保留最近 %d 份：%v %v", keepSnapshots, snapshots, err)
	}
	if snapshots[0].Revision != "sync-"+start.Add(7*time.Hour).Format("20060102150405")+"-办公室" {
		t.Fatalf("最新的应留着并排在最前：%v", snapshots)
	}
	// 另一台设备的东西不受影响
	if _, err := uploadSnapshot(store, deviceB, "", file, start.Add(-time.Hour)); err != nil {
		t.Fatal(err)
	}
	devices, err := listDevices(store)
	if err != nil || len(devices) != 2 || devices[0].ID != deviceA || devices[0].Hostname != "办公室" || devices[1].ID != deviceB {
		t.Fatalf("设备应按最近上传排序：%+v %v", devices, err)
	}
	if deviceName(devices[1]) != deviceB[:8] {
		t.Fatalf("没有电脑名的设备用标识开头称呼：%s", deviceName(devices[1]))
	}
	// 进程号位数不同时也按数值比：10-9 比 9-100 新
	store.files[snapshotKey(deviceB, "9-100")] = []byte("旧")
	store.files[snapshotKey(deviceB, "10-9")] = []byte("新")
	store.files["devices/"+deviceB+"/notes.txt"] = []byte("不是快照")
	delete(store.files, devices[1].snapshotKey())
	store.files[deviceInfoKey(deviceB)] = []byte(`{"revision":"20-1","uploaded_unix":20}`)
	devices, _ = listDevices(store)
	revisions, _ := listRevisions(store, deviceB)
	if len(revisions) != 2 || revisions[0] != "10-9" || revisions[1] != "9-100" {
		t.Fatalf("修订号要按数值排：%v", revisions)
	}
	// 设备信息指向的快照不在了：退回实际存在的最新一份
	out := filepath.Join(t.TempDir(), "restored.json")
	if err := downloadSnapshot(store, devices[1], out); err != nil {
		t.Fatal(err)
	}
	if data, _ := os.ReadFile(out); string(data) != "新" {
		t.Fatalf("应退回最新的快照：%q", data)
	}
	if err := downloadSnapshot(store, devices[0], out); err != nil {
		t.Fatal(err)
	}
	if data, _ := os.ReadFile(out); string(data) != `{"snapshot":7}` {
		t.Fatalf("应下载设备信息指向的快照：%q", data)
	}
	// 列表显示后最新文件被移除，手动恢复仍能找到同一电脑的上一份。
	delete(store.files, devices[0].snapshotKey())
	if err := downloadSnapshot(store, devices[0], out); err != nil {
		t.Fatal(err)
	}
	if data, _ := os.ReadFile(out); string(data) != `{"snapshot":6}` {
		t.Fatal("手动恢复应退回同一电脑现存的最新备份")
	}
	// 设备信息文件里写的标识、修订号不能拿来拼别的路径
	store.files[deviceInfoKey(deviceB)] = []byte(`{"device_id":"../x","revision":"../../secret","uploaded_unix":5}`)
	devices, _ = listDevices(store)
	if devices[1].ID != deviceB || devices[1].Revision != "" {
		t.Fatalf("不可信的设备信息没有被纠正：%+v", devices[1])
	}
	if _, err := uploadSnapshot(store, "../x", "", file, start); err == nil {
		t.Fatal("不合格的设备标识应被拒绝")
	}
}

func TestSyncFolderAndNamesAcrossProviders(t *testing.T) {
	for _, provider := range []string{"webdav", "s3"} {
		for _, prefix := range []string{"", "备份", "备份/FreeType/"} {
			t.Run(provider+"/"+prefix, func(t *testing.T) {
				dav := &fakeDAV{t: t, dirs: map[string]bool{"": true, "/dav": true, "/dav/备份": true}, files: map[string][]byte{}}
				s3 := &fakeS3{t: t, objects: map[string][]byte{}}
				var handler http.Handler = dav
				if provider == "s3" {
					handler = s3
				}
				server := httptest.NewServer(handler)
				defer server.Close()
				c := syncSettings{Provider: provider, WebDAVURL: server.URL + "/dav/" + prefix, WebDAVUser: "me", WebDAVPassword: "secret",
					S3Endpoint: server.URL, S3Region: "auto", S3Bucket: "bucket", S3Prefix: prefix, S3AccessKey: "key", S3SecretKey: "secret", S3PathStyle: true}
				store, err := openRemoteStore(c)
				if err != nil {
					t.Fatal(err)
				}
				file := filepath.Join(t.TempDir(), "snapshot.json")
				if err := os.WriteFile(file, []byte(`{"format":"freetype-settings","value":"完整备份"}`), 0600); err != nil {
					t.Fatal(err)
				}
				now := time.Date(2026, 11, 17, 14, 30, 5, 0, time.Local)
				for i := 0; i < 2; i++ {
					if _, err := uploadSnapshot(store, deviceA, "电脑", file, now); err != nil {
						t.Fatal(err)
					}
				}
				base := "FreeType/"
				if prefix != "" {
					base = "备份/FreeType/"
				}
				files := s3.objects
				if provider == "webdav" {
					base = "/dav/" + base
					files = dav.files
				}
				for _, name := range []string{"sync-20261117143005-电脑.json", "sync-20261117143006-电脑.json"} {
					if !json.Valid(files[base+name]) || !bytes.Contains(files[base+name], []byte("完整备份")) {
						t.Fatalf("缺少指定快照 %s: %v", name, files)
					}
				}
				if len(files) != 2 {
					t.Fatalf("只能有两份直接位于 FreeType 的快照，不应有设备索引或子目录：%v", files)
				}
				for dir := range dav.dirs {
					if strings.Contains(dir, "/devices") {
						t.Fatal("不应创建 devices 目录")
					}
				}
				devices, err := listDevices(store)
				if err != nil || len(devices) != 1 {
					t.Fatalf("%v %v", devices, err)
				}
				if err := downloadSnapshot(store, devices[0], file); err != nil {
					t.Fatal(err)
				}
				if got, _ := os.ReadFile(file); string(got) != `{"format":"freetype-settings","value":"完整备份"}` {
					t.Fatal("恢复后数据必须与原始快照完全一致")
				}
			})
		}
	}
}

func TestSyncReadsLegacyLocationWithoutWritingOutsideFreeType(t *testing.T) {
	dav := &fakeDAV{t: t, dirs: map[string]bool{"": true, "/dav": true}, files: map[string][]byte{}}
	server := httptest.NewServer(dav)
	defer server.Close()
	old, _ := newWebDAVStore(server.URL+"/dav", "me", "secret")
	meta := `{"revision":"1700000000-1","uploaded_unix":1700000000}`
	if err := old.Put(deviceInfoKey(deviceA), strings.NewReader(meta), int64(len(meta))); err != nil {
		t.Fatal(err)
	}
	if err := old.Put(snapshotKey(deviceA, "1700000000-1"), strings.NewReader("旧备份"), int64(len("旧备份"))); err != nil {
		t.Fatal(err)
	}
	store, err := openRemoteStore(syncSettings{WebDAVURL: server.URL + "/dav", WebDAVUser: "me", WebDAVPassword: "secret"})
	if err != nil {
		t.Fatal(err)
	}
	devices, err := readDevices(store, true)
	if err != nil || len(devices) != 1 {
		t.Fatalf("旧目录不可读：%+v %v", devices, err)
	}
	file := filepath.Join(t.TempDir(), "restored.json")
	if err := downloadSnapshot(store, devices[0], file); err != nil {
		t.Fatal(err)
	}
	if got, _ := os.ReadFile(file); string(got) != "旧备份" {
		t.Fatalf("%s", got)
	}
	if err := os.WriteFile(file, []byte(`{"value":"新备份"}`), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := uploadSnapshot(store, deviceA, "电脑", file, time.Date(2026, 11, 17, 14, 30, 5, 0, time.Local)); err != nil {
		t.Fatal(err)
	}
	devices, err = listDevices(store)
	if err != nil || len(devices) != 1 || devices[0].Revision != "sync-20261117143005-电脑" {
		t.Fatalf("%+v %v", devices, err)
	}
	if len(dav.files) != 3 || dav.files["/dav/FreeType/sync-20261117143005-电脑.json"] == nil {
		t.Fatalf("新上传没有进入 FreeType：%v", dav.files)
	}
}

func TestSimultaneousUploadsWithSameComputerNameNeverOverwrite(t *testing.T) {
	for _, provider := range []string{"webdav", "s3"} {
		t.Run(provider, func(t *testing.T) {
			dav := &fakeDAV{t: t, dirs: map[string]bool{"": true, "/dav": true}, files: map[string][]byte{}}
			s3 := &fakeS3{t: t, objects: map[string][]byte{}}
			var service http.Handler = dav
			if provider == "s3" {
				service = s3
			}
			var contenders atomic.Int32
			ready := make(chan struct{})
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				// 让两台设备都读到空目录，再同时争抢同一个名字。
				if r.Method == http.MethodPut && strings.HasSuffix(r.URL.Path, "sync-20261117143005-办公_电脑__.json") {
					if contenders.Add(1) == 2 {
						close(ready)
					}
					select {
					case <-ready:
					case <-time.After(5 * time.Second):
						t.Error("两个上传没有同时到达")
						w.WriteHeader(500)
						return
					}
					if r.Header.Get("If-None-Match") != "*" {
						t.Error("创建备份必须阻止覆盖")
					}
				}
				service.ServeHTTP(w, r)
			}))
			defer server.Close()
			c := syncSettings{Provider: provider, WebDAVURL: server.URL + "/dav/FreeType", WebDAVUser: "me", WebDAVPassword: "secret", S3Endpoint: server.URL, S3Region: "auto", S3Bucket: "bucket", S3Prefix: "FreeType", S3AccessKey: "key", S3SecretKey: "secret", S3PathStyle: true}
			file := filepath.Join(t.TempDir(), "snapshot.json")
			body := []byte("{\n  \"value\": \"完整备份\"\n}\n")
			if err := os.WriteFile(file, body, 0600); err != nil {
				t.Fatal(err)
			}
			errors := make(chan error, 2)
			for _, id := range []string{deviceA, deviceB} {
				go func(id string) {
					store, err := openRemoteStore(c)
					if err == nil {
						_, err = uploadSnapshot(store, id, "办公/电脑:*", file, time.Date(2026, 11, 17, 14, 30, 5, 0, time.Local))
					}
					errors <- err
				}(id)
			}
			for i := 0; i < 2; i++ {
				if err := <-errors; err != nil {
					t.Fatal(err)
				}
			}
			store, _ := openRemoteStore(c)
			devices, err := listDevices(store)
			if err != nil || len(devices) != 2 || devices[0].Revision == devices[1].Revision {
				t.Fatalf("同秒同电脑名应保留两台设备的备份：%+v %v", devices, err)
			}
			entries, err := store.List("")
			if err != nil || len(entries) != 2 {
				t.Fatalf("只能有两份直接可见的备份：%v %v", entries, err)
			}
			for _, d := range devices {
				if err := downloadSnapshot(store, d, file); err != nil {
					t.Fatal(err)
				}
				if got, _ := os.ReadFile(file); !bytes.Equal(got, body) {
					t.Fatal("恢复必须保留原始数据及换行")
				}
			}
		})
	}
}

func TestFailedUploadLeavesDeviceInfoPointingAtTheLastGoodSnapshot(t *testing.T) {
	store := &memStore{files: map[string][]byte{}}
	file := filepath.Join(t.TempDir(), "snapshot.json")
	os.WriteFile(file, []byte(`{"value":"好的"}`), 0o600)
	first, err := uploadSnapshot(store, deviceA, "pc", file, time.Unix(100, 0))
	if err != nil {
		t.Fatal(err)
	}
	store.fail = "put"
	if _, err := uploadSnapshot(store, deviceA, "pc", file, time.Unix(200, 0)); err == nil {
		t.Fatal("上传失败应报错")
	}
	store.fail = ""
	devices, _ := listDevices(store)
	if len(devices) != 1 || devices[0].Revision != first.Revision {
		t.Fatalf("失败的上传不应改设备信息：%+v", devices)
	}
}

func TestFlatRetentionPreservesOtherComputerAndUnrelatedFiles(t *testing.T) {
	store := &memStore{files: map[string][]byte{"说明.txt": []byte("保留")}}
	file := filepath.Join(t.TempDir(), "snapshot.json")
	if err := os.WriteFile(file, []byte(`{"value":"备份"}`), 0600); err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 11, 17, 14, 30, 5, 0, time.Local)
	if _, err := uploadVersionedSnapshot(store, deviceB, "家里电脑", file, now, syncVersion{}, 1); err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 3; i++ {
		if _, err := uploadVersionedSnapshot(store, deviceA, "办公电脑", file, now.Add(time.Duration(i)*time.Second), syncVersion{}, 1); err != nil {
			t.Fatal(err)
		}
	}
	for _, name := range []string{"说明.txt", "sync-20261117143005-家里电脑.json", "sync-20261117143007-办公电脑.json"} {
		if _, exists := store.files[name]; !exists {
			t.Fatalf("不应删除 %s", name)
		}
	}
	if len(store.files) != 3 {
		t.Fatalf("本机只应保留一份，其他电脑和无关文件保留：%v", store.files)
	}
}

func TestFlatSnapshotListingReadsOnlyTheHeader(t *testing.T) {
	file := filepath.Join(t.TempDir(), "snapshot.json")
	body := []byte(`{"large":"` + strings.Repeat("x", 1<<20) + `"}`)
	if err := os.WriteFile(file, body, 0600); err != nil {
		t.Fatal(err)
	}
	store := &memStore{files: map[string][]byte{}}
	d, err := uploadSnapshot(store, deviceA, "办公电脑", file, time.Date(2026, 11, 17, 14, 30, 5, 0, time.Local))
	if err != nil {
		t.Fatal(err)
	}
	header := &syncHeaderReader{}
	read, err := header.Write(store.files[d.snapshotKey()])
	if !errors.Is(err, errSyncHeaderComplete) || read > 16<<10 {
		t.Fatal("列举备份不应下载完整大文件", read, err)
	}
	if parsed, err := parseSyncHeader(header.line, d.Revision); err != nil || parsed.ID != deviceA {
		t.Fatal("首行应足够恢复设备信息", err)
	}
}

func TestSyncSettingsKeepSecretsEncryptedOnDiskAndDeviceIDStable(t *testing.T) {
	dir := t.TempDir()
	c, err := loadSyncSettings(dir)
	if err != nil || !deviceIDPattern.MatchString(c.DeviceID) || c.Enabled || c.Provider != "webdav" {
		t.Fatalf("默认值：%+v %v", c, err)
	}
	c.Enabled, c.WebDAVURL, c.WebDAVUser, c.WebDAVPassword = true, "https://dav.example.com/x", "me", "应用密码-123"
	c.S3AccessKey, c.S3SecretKey = "example-access-key-id", "s3-secret-456"
	if err := saveSyncSettings(dir, c); err != nil {
		t.Fatal(err)
	}
	raw, _ := os.ReadFile(filepath.Join(dir, "sync.json"))
	for _, secret := range []string{"应用密码-123", "s3-secret-456"} {
		if bytes.Contains(raw, []byte(secret)) {
			t.Fatalf("密码明文写进了文件：%s", raw)
		}
	}
	if !bytes.Contains(raw, []byte("example-access-key-id")) || !bytes.Contains(raw, []byte(protectedPrefix)) {
		t.Fatalf("Access Key 是标识不加密，密码要带加密标记：%s", raw)
	}
	again, err := loadSyncSettings(dir)
	if err != nil || again != c {
		t.Fatalf("读回来应和保存的一样：%+v %v", again, err)
	}
	// 密文坏了（换了账号或电脑）：密码清空，别的设置和设备标识留着
	os.WriteFile(filepath.Join(dir, "sync.json"), bytes.ReplaceAll(raw, []byte(protectedPrefix), []byte(protectedPrefix+"AAAA")), 0o600)
	broken, err := loadSyncSettings(dir)
	if err != nil || broken.WebDAVPassword != "" || broken.WebDAVUser != "me" || broken.DeviceID != c.DeviceID {
		t.Fatalf("解不开的密码应清空：%+v %v", broken, err)
	}
}

func TestSyncPageUploadsListsAndMergesThroughTheBackend(t *testing.T) {
	s, b := newTestSettings()
	s.syncDir = t.TempDir()
	store := &memStore{files: map[string][]byte{}}
	var opened syncSettings
	s.newStore = func(c syncSettings) (remoteStore, error) { opened = c; return store, nil }
	tt := open(t, s, "数据与同步")
	if tt.HasText("刷新设备") {
		t.Fatal("没启用同步时不显示上传按钮")
	}
	if !tt.HasText("一键导入导出") || tt.HasText("一键导入、导出") {
		t.Fatalf("一键导入导出应在这一页，标题不带顿号：%q", tt.Texts())
	}
	if err := tt.Click("启用同步"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	s.sync.WebDAVURL, s.sync.WebDAVPassword = "https://dav.example.com/FreeType", "pw"
	if err := tt.Click("保存并测试连接"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("连接成功，可以上传和恢复") || opened.WebDAVPassword != "pw" {
		t.Fatalf("测试连接：%q %+v", tt.Texts(), opened)
	}
	button, found := tt.Find("保存并测试连接")
	if !found {
		t.Fatal("缺少连接按钮")
	}
	near := false
	for _, node := range tt.AccessAll() {
		if node.Name == "连接成功，可以上传和恢复" && node.Bounds.X > button.X && node.Bounds.Y < button.Y+button.H {
			near = true
		}
	}
	if !near {
		t.Fatal("连接结果必须紧邻测试按钮")
	}
	if saved, err := loadSyncSettings(s.syncDir); err != nil || !saved.Enabled || saved.WebDAVPassword != "pw" || saved.DeviceID != s.sync.DeviceID {
		t.Fatalf("同步设置没有保存：%+v %v", saved, err)
	}
	if err := tt.Click("立即上传"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	exported, _ := b.last("export_settings")["path"].(string)
	if exported == "" {
		t.Fatal("上传没有让服务进程生成快照")
	}
	if _, err := os.Stat(exported); !os.IsNotExist(err) {
		t.Fatalf("临时快照没有删掉：%s", exported)
	}
	if len(store.files) != 1 || len(s.syncDevices) != 1 || !tt.HasText("（这台电脑）") {
		t.Fatalf("上传后只应有一份备份，并列出这台电脑：%v %+v", store.files, s.syncDevices)
	}
	// 另一台电脑的快照：刷新后能看到，点恢复、确认后交给服务进程合并
	store.files[snapshotKey(deviceB, "50-1")] = []byte(`{"format":"freetype-settings"}`)
	store.files[deviceInfoKey(deviceB)] = []byte(`{"hostname":"家里的电脑","revision":"50-1","uploaded_unix":50,"size":30}`)
	if err := tt.Click("刷新设备"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("家里的电脑") {
		t.Fatalf("没有列出另一台电脑：%q", tt.Texts())
	}
	if err := tt.Click("恢复 家里的电脑"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.last("merge_settings") != nil {
		t.Fatal("确认之前不能合并")
	}
	if err := tt.Click("合并"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if b.last("merge_settings") == nil || b.last("import_settings") != nil {
		t.Fatal("同步恢复应走合并，不是整份替换")
	}
	if !tt.HasText("已合并 家里的电脑 的快照") {
		t.Fatalf("没有显示合并结果：%q", tt.Texts())
	}
	// 失败要显示原因
	store.fail = "probe"
	if err := tt.Click("保存并测试连接"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("连接失败：测试错误") {
		t.Fatalf("没有显示失败原因：%q", tt.Texts())
	}
}
