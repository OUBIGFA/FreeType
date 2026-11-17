package main

// 数据同步的传输层：把文件放到用户自己的 WebDAV 或 S3 兼容存储上。
//
// 只用标准库：WebDAV 是几个 HTTP 方法，S3 的 SigV4 签名是公开算法。TLS 校验始终开启，
// 自签证书的服务端应把证书导入系统信任库；代理跟随系统环境变量。

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"sort"
	"strings"
	"time"
)

// remoteEntry 是远端某一层目录里的一项。
type remoteEntry struct {
	Name string
	Dir  bool
	Size int64
}

// remoteStore 是同步用到的远端操作。key 是相对同步根目录的路径，用 / 分段。
type remoteStore interface {
	// Put 上传；body 要能回到开头（S3 签名前先算一遍摘要）。
	Put(key string, body io.ReadSeeker, size int64) error
	Get(key string, w io.Writer) error
	// List 只列 dir 这一层；目录不存在时返回空。
	List(dir string) ([]remoteEntry, error)
	// Delete 删除；本来就没有也算成功。
	Delete(key string) error
	// Probe 检查地址和凭据能不能用。
	Probe() error
}

// 等服务端开始应答最多这么久；传输本身不设总时长（快照可能很大、网可能很慢）。
const remoteHeaderTimeout = 30 * time.Second

func remoteClient(followRedirects bool) *http.Client {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.ResponseHeaderTimeout = remoteHeaderTimeout
	client := &http.Client{Transport: transport}
	if !followRedirects {
		client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	}
	return client
}

// remoteError 把失败的应答写成一句话（带服务端给的原因，最多一小段）。
func remoteError(action string, resp *http.Response) error {
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 2048))
	detail := strings.TrimSpace(string(body))
	var s3 struct {
		Code    string `xml:"Code"`
		Message string `xml:"Message"`
	}
	if xml.Unmarshal(body, &s3) == nil && s3.Code != "" {
		detail = s3.Code + "：" + s3.Message
	}
	switch resp.StatusCode {
	case http.StatusUnauthorized, http.StatusForbidden:
		return fmt.Errorf("%s失败：账号、密码或密钥不对，或者没有权限（HTTP %d）%s", action, resp.StatusCode, short(detail))
	case http.StatusInsufficientStorage:
		return fmt.Errorf("%s失败：远端空间不足（HTTP 507）", action)
	}
	return fmt.Errorf("%s失败：HTTP %d %s", action, resp.StatusCode, short(detail))
}

func short(detail string) string {
	if detail == "" || strings.HasPrefix(detail, "<") {
		return ""
	}
	if r := []rune(detail); len(r) > 160 {
		detail = string(r[:160]) + "…"
	}
	return "：" + detail
}

func cleanKey(key string) []string {
	var parts []string
	for _, part := range strings.Split(key, "/") {
		if part != "" {
			parts = append(parts, part)
		}
	}
	return parts
}

// ---------- WebDAV ----------

type webdavStore struct {
	base     *url.URL
	user     string
	password string
	client   *http.Client
	// 已经确认存在的目录，免得每次上传都重建一遍。
	made map[string]bool
}

func newWebDAVStore(address, user, password string) (*webdavStore, error) {
	base, err := url.Parse(strings.TrimSpace(address))
	if err != nil || base.Host == "" || (base.Scheme != "https" && base.Scheme != "http") {
		return nil, errors.New("WebDAV 地址要以 https:// 或 http:// 开头，例如 https://dav.jianguoyun.com/dav/FreeType")
	}
	base.Path = strings.TrimRight(base.Path, "/")
	base.RawPath, base.RawQuery, base.Fragment = "", "", ""
	return &webdavStore{base: base, user: user, password: password, client: remoteClient(true), made: map[string]bool{}}, nil
}

// address 拼出 key 的地址。路径按解码后的文字存，发请求时统一编码一次（目录名可以是中文）。
func (w *webdavStore) address(parts []string, dir bool) string {
	u := *w.base
	u.Path = w.base.Path + "/" + strings.Join(parts, "/")
	if len(parts) == 0 {
		u.Path = w.base.Path
	}
	if dir {
		u.Path += "/"
	}
	return u.String()
}

func (w *webdavStore) do(method, address string, body io.Reader, size int64, headers map[string]string) (*http.Response, error) {
	req, err := http.NewRequest(method, address, body)
	if err != nil {
		return nil, err
	}
	if body != nil {
		// 总是带上长度，不用分块传输（不少 WebDAV 服务端不认）
		req.ContentLength = size
	}
	if w.user != "" || w.password != "" {
		req.SetBasicAuth(w.user, w.password)
	}
	for name, value := range headers {
		req.Header.Set(name, value)
	}
	return w.client.Do(req)
}

// ensureDirs 逐级建目录：往不存在的目录里 PUT 会得到 409。
func (w *webdavStore) ensureDirs(parts []string) error {
	for i := 0; i <= len(parts); i++ {
		address := w.address(parts[:i], true)
		if w.made[address] {
			continue
		}
		resp, err := w.do("MKCOL", address, nil, 0, nil)
		if err != nil {
			return fmt.Errorf("创建远端目录失败：%w", err)
		}
		// 201 建好了；405 已经有了
		exists := resp.StatusCode == http.StatusCreated || resp.StatusCode == http.StatusMethodNotAllowed ||
			resp.StatusCode == http.StatusOK || resp.StatusCode == http.StatusNoContent
		// 同步根目录建不出来（有的服务端不让建，或上级目录不存在）先不报错：
		// 它本来就在的话后面的请求照样能用，不在的话后面的请求会给出确切原因
		if !exists && (i > 0 || resp.StatusCode == http.StatusUnauthorized) {
			err = remoteError("创建远端目录", resp)
		}
		resp.Body.Close()
		if err != nil {
			return err
		}
		if exists {
			w.made[address] = true
		}
	}
	return nil
}

func (w *webdavStore) Put(key string, body io.ReadSeeker, size int64) error {
	parts := cleanKey(key)
	if err := w.ensureDirs(parts[:len(parts)-1]); err != nil {
		return err
	}
	resp, err := w.do(http.MethodPut, w.address(parts, false), body, size, map[string]string{"Content-Type": "application/octet-stream"})
	if err != nil {
		return fmt.Errorf("上传失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 {
		return remoteError("上传", resp)
	}
	return nil
}

func (w *webdavStore) Get(key string, out io.Writer) error {
	resp, err := w.do(http.MethodGet, w.address(cleanKey(key), false), nil, 0, nil)
	if err != nil {
		return fmt.Errorf("下载失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return remoteError("下载", resp)
	}
	// 传到一半断了要算失败，不能只看开头的 200
	if _, err := io.Copy(out, resp.Body); err != nil {
		return fmt.Errorf("下载中断：%w", err)
	}
	return nil
}

const propfindBody = `<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/><D:getcontentlength/></D:prop></D:propfind>`

type davMultistatus struct {
	Responses []struct {
		Href  string `xml:"href"`
		Props []struct {
			Collection *struct{} `xml:"prop>resourcetype>collection"`
			Length     string    `xml:"prop>getcontentlength"`
		} `xml:"propstat"`
	} `xml:"response"`
}

func (w *webdavStore) propfind(address, depth string) (*http.Response, error) {
	return w.do("PROPFIND", address, strings.NewReader(propfindBody), int64(len(propfindBody)),
		map[string]string{"Depth": depth, "Content-Type": "application/xml; charset=utf-8"})
}

// hrefPath 取出 href 里解码后的路径（服务端可能回完整地址，也可能只回路径）。
func hrefPath(href string) string {
	if u, err := url.Parse(strings.TrimSpace(href)); err == nil {
		return strings.TrimRight(u.Path, "/")
	}
	return strings.TrimRight(href, "/")
}

func (w *webdavStore) List(dir string) ([]remoteEntry, error) {
	parts := cleanKey(dir)
	address := w.address(parts, true)
	resp, err := w.propfind(address, "1")
	if err != nil {
		return nil, fmt.Errorf("读取远端目录失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusNotFound {
		return nil, nil
	}
	if resp.StatusCode != http.StatusMultiStatus {
		return nil, remoteError("读取远端目录", resp)
	}
	var result davMultistatus
	if err := xml.NewDecoder(io.LimitReader(resp.Body, 8<<20)).Decode(&result); err != nil {
		return nil, fmt.Errorf("远端目录的应答看不懂：%w", err)
	}
	self := hrefPath(address)
	var entries []remoteEntry
	for _, item := range result.Responses {
		path := hrefPath(item.Href)
		// 两边都解码后再比：目录名有中文时服务端回的编码写法不一定和请求一样
		if path == self || !strings.HasPrefix(path, self+"/") {
			continue
		}
		name := path[len(self)+1:]
		if name == "" || strings.Contains(name, "/") {
			continue
		}
		entry := remoteEntry{Name: name}
		for _, prop := range item.Props {
			entry.Dir = entry.Dir || prop.Collection != nil
			if prop.Length != "" {
				fmt.Sscan(strings.TrimSpace(prop.Length), &entry.Size)
			}
		}
		entries = append(entries, entry)
	}
	return entries, nil
}

func (w *webdavStore) Delete(key string) error {
	resp, err := w.do(http.MethodDelete, w.address(cleanKey(key), false), nil, 0, nil)
	if err != nil {
		return fmt.Errorf("删除失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 && resp.StatusCode != http.StatusNotFound {
		return remoteError("删除", resp)
	}
	return nil
}

func (w *webdavStore) Probe() error {
	if err := w.ensureDirs(nil); err != nil {
		return err
	}
	resp, err := w.propfind(w.address(nil, true), "0")
	if err != nil {
		return fmt.Errorf("连接失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusMultiStatus {
		if resp.StatusCode == http.StatusNotFound {
			return errors.New("连接失败：远端目录不存在，也建不出来。请先在网盘里建好这个文件夹")
		}
		return remoteError("连接", resp)
	}
	return nil
}

// ---------- S3 兼容 ----------

type s3Store struct {
	endpoint  *url.URL
	region    string
	bucket    string
	prefix    []string
	accessKey string
	secretKey string
	// 路径风格：<endpoint>/<bucket>/<key>（MinIO 等）；否则是 <bucket>.<endpoint>/<key>。
	pathStyle bool
	client    *http.Client
	now       func() time.Time
}

func newS3Store(c syncSettings) (*s3Store, error) {
	address := strings.TrimSpace(c.S3Endpoint)
	if address != "" && !strings.Contains(address, "://") {
		address = "https://" + address
	}
	endpoint, err := url.Parse(address)
	if err != nil || endpoint.Host == "" || (endpoint.Scheme != "https" && endpoint.Scheme != "http") {
		return nil, errors.New("S3 服务地址不对，例如 https://s3.amazonaws.com 或 https://minio.example.com:9000")
	}
	bucket := strings.TrimSpace(c.S3Bucket)
	if bucket == "" || strings.ContainsAny(bucket, "/ ") {
		return nil, errors.New("请填写存储桶名称")
	}
	if strings.TrimSpace(c.S3AccessKey) == "" || c.S3SecretKey == "" {
		return nil, errors.New("请填写 Access Key 和 Secret Key")
	}
	region := strings.TrimSpace(c.S3Region)
	if region == "" {
		region = "us-east-1"
	}
	endpoint.Path, endpoint.RawPath, endpoint.RawQuery, endpoint.Fragment = "", "", "", ""
	return &s3Store{
		endpoint: endpoint, region: region, bucket: bucket, prefix: cleanKey(c.S3Prefix),
		accessKey: strings.TrimSpace(c.S3AccessKey), secretKey: c.S3SecretKey, pathStyle: c.S3PathStyle,
		// 不跟随重定向：签名绑定了主机名，跳过去必然验签失败
		client: remoteClient(false), now: time.Now,
	}, nil
}

// s3Escape 按 SigV4 的规则编码：只留字母、数字和 - _ . ~。
func s3Escape(text string) string {
	var out strings.Builder
	for _, b := range []byte(text) {
		if b >= 'A' && b <= 'Z' || b >= 'a' && b <= 'z' || b >= '0' && b <= '9' || b == '-' || b == '_' || b == '.' || b == '~' {
			out.WriteByte(b)
		} else {
			fmt.Fprintf(&out, "%%%02X", b)
		}
	}
	return out.String()
}

func s3Path(parts []string) string {
	escaped := make([]string, len(parts))
	for i, part := range parts {
		escaped[i] = s3Escape(part)
	}
	return "/" + strings.Join(escaped, "/")
}

func s3Query(query map[string]string) string {
	names := make([]string, 0, len(query))
	for name := range query {
		names = append(names, name)
	}
	sort.Strings(names)
	pairs := make([]string, len(names))
	for i, name := range names {
		pairs[i] = s3Escape(name) + "=" + s3Escape(query[name])
	}
	return strings.Join(pairs, "&")
}

func hmacSHA256(key []byte, text string) []byte {
	mac := hmac.New(sha256.New, key)
	mac.Write([]byte(text))
	return mac.Sum(nil)
}

// sigV4 算 Authorization 头。headers 是要签名的头（名字小写），必须含 host、x-amz-date、
// x-amz-content-sha256；path、query 已经按规则编码好。
func sigV4(method, path, query string, headers map[string]string, payloadHash, region, accessKey, secretKey string) string {
	names := make([]string, 0, len(headers))
	for name := range headers {
		names = append(names, name)
	}
	sort.Strings(names)
	var canonicalHeaders strings.Builder
	for _, name := range names {
		canonicalHeaders.WriteString(name + ":" + strings.TrimSpace(headers[name]) + "\n")
	}
	signed := strings.Join(names, ";")
	canonical := strings.Join([]string{method, path, query, canonicalHeaders.String(), signed, payloadHash}, "\n")
	stamp := headers["x-amz-date"]
	scope := stamp[:8] + "/" + region + "/s3/aws4_request"
	digest := sha256.Sum256([]byte(canonical))
	toSign := strings.Join([]string{"AWS4-HMAC-SHA256", stamp, scope, hex.EncodeToString(digest[:])}, "\n")
	key := hmacSHA256([]byte("AWS4"+secretKey), stamp[:8])
	key = hmacSHA256(key, region)
	key = hmacSHA256(key, "s3")
	key = hmacSHA256(key, "aws4_request")
	signature := hex.EncodeToString(hmacSHA256(key, toSign))
	return "AWS4-HMAC-SHA256 Credential=" + accessKey + "/" + scope + ", SignedHeaders=" + signed + ", Signature=" + signature
}

const emptySHA256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

// do 发一个签好名的请求。parts 是桶里的对象路径（已含同步根目录）。
func (s *s3Store) do(method string, parts []string, query map[string]string, body io.ReadSeeker, size int64) (*http.Response, error) {
	host := s.endpoint.Host
	path := s3Path(parts)
	if s.pathStyle {
		path = s3Path(append([]string{s.bucket}, parts...))
	} else {
		host = s.bucket + "." + host
	}
	payloadHash := emptySHA256
	if body != nil {
		hash := sha256.New()
		if _, err := io.Copy(hash, body); err != nil {
			return nil, err
		}
		if _, err := body.Seek(0, io.SeekStart); err != nil {
			return nil, err
		}
		payloadHash = hex.EncodeToString(hash.Sum(nil))
	}
	rawQuery := s3Query(query)
	address := s.endpoint.Scheme + "://" + host + path
	if rawQuery != "" {
		address += "?" + rawQuery
	}
	var reader io.Reader
	if body != nil {
		reader = body
	}
	req, err := http.NewRequest(method, address, reader)
	if err != nil {
		return nil, err
	}
	// 地址已按签名规则编码；net/url 解析后原样保留这份编码（RawPath、RawQuery），发出去的和签名的一致
	if body != nil {
		req.ContentLength = size
	}
	headers := map[string]string{
		"host":                 host,
		"x-amz-date":           s.now().UTC().Format("20060102T150405Z"),
		"x-amz-content-sha256": payloadHash,
	}
	req.Header.Set("x-amz-date", headers["x-amz-date"])
	req.Header.Set("x-amz-content-sha256", payloadHash)
	req.Header.Set("Authorization", sigV4(method, path, rawQuery, headers, payloadHash, s.region, s.accessKey, s.secretKey))
	resp, err := s.client.Do(req)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode/100 == 3 {
		defer resp.Body.Close()
		hint := "区域或服务地址不对"
		if region := resp.Header.Get("x-amz-bucket-region"); region != "" {
			hint += "，这个存储桶在 " + region
		}
		return nil, fmt.Errorf("%s（HTTP %d）", hint, resp.StatusCode)
	}
	return resp, nil
}

func (s *s3Store) object(key string) []string {
	return append(append([]string{}, s.prefix...), cleanKey(key)...)
}

func (s *s3Store) Put(key string, body io.ReadSeeker, size int64) error {
	resp, err := s.do(http.MethodPut, s.object(key), nil, body, size)
	if err != nil {
		return fmt.Errorf("上传失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 {
		return remoteError("上传", resp)
	}
	return nil
}

func (s *s3Store) Get(key string, out io.Writer) error {
	resp, err := s.do(http.MethodGet, s.object(key), nil, nil, 0)
	if err != nil {
		return fmt.Errorf("下载失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return remoteError("下载", resp)
	}
	if _, err := io.Copy(out, resp.Body); err != nil {
		return fmt.Errorf("下载中断：%w", err)
	}
	return nil
}

type s3Listing struct {
	Contents []struct {
		Key  string `xml:"Key"`
		Size int64  `xml:"Size"`
	} `xml:"Contents"`
	CommonPrefixes []struct {
		Prefix string `xml:"Prefix"`
	} `xml:"CommonPrefixes"`
	IsTruncated bool   `xml:"IsTruncated"`
	Next        string `xml:"NextContinuationToken"`
}

func (s *s3Store) list(dir string, limit int) ([]remoteEntry, error) {
	prefix := strings.Join(s.object(dir), "/")
	if prefix != "" {
		prefix += "/"
	}
	var entries []remoteEntry
	token := ""
	for {
		// 带 delimiter：和 WebDAV 一样只列一层
		query := map[string]string{"list-type": "2", "prefix": prefix, "delimiter": "/"}
		if limit > 0 {
			query["max-keys"] = fmt.Sprint(limit)
		}
		if token != "" {
			query["continuation-token"] = token
		}
		resp, err := s.do(http.MethodGet, nil, query, nil, 0)
		if err != nil {
			return nil, fmt.Errorf("读取远端目录失败：%w", err)
		}
		if resp.StatusCode != http.StatusOK {
			err = remoteError("读取远端目录", resp)
			resp.Body.Close()
			return nil, err
		}
		var page s3Listing
		err = xml.NewDecoder(io.LimitReader(resp.Body, 8<<20)).Decode(&page)
		resp.Body.Close()
		if err != nil {
			return nil, fmt.Errorf("远端目录的应答看不懂：%w", err)
		}
		for _, item := range page.Contents {
			if name := strings.TrimPrefix(item.Key, prefix); name != "" && !strings.Contains(name, "/") {
				entries = append(entries, remoteEntry{Name: name, Size: item.Size})
			}
		}
		for _, item := range page.CommonPrefixes {
			if name := strings.Trim(strings.TrimPrefix(item.Prefix, prefix), "/"); name != "" {
				entries = append(entries, remoteEntry{Name: name, Dir: true})
			}
		}
		if !page.IsTruncated || page.Next == "" || limit > 0 {
			return entries, nil
		}
		token = page.Next
	}
}

func (s *s3Store) List(dir string) ([]remoteEntry, error) { return s.list(dir, 0) }

func (s *s3Store) Delete(key string) error {
	resp, err := s.do(http.MethodDelete, s.object(key), nil, nil, 0)
	if err != nil {
		return fmt.Errorf("删除失败：%w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 && resp.StatusCode != http.StatusNotFound {
		return remoteError("删除", resp)
	}
	return nil
}

func (s *s3Store) Probe() error {
	if _, err := s.list("", 1); err != nil {
		return fmt.Errorf("连接失败：%s", strings.TrimPrefix(err.Error(), "读取远端目录失败："))
	}
	// 能列不代表能写：放一个很小的对象再删掉
	probe := []byte("FreeType")
	if err := s.Put(".freetype-probe", bytes.NewReader(probe), int64(len(probe))); err != nil {
		return fmt.Errorf("能读取存储桶，但写不进去：%w", err)
	}
	return s.Delete(".freetype-probe")
}
