package main

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/pbkdf2"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"io"
	"strings"
)

// 云端备份加密（默认关闭，在「云端同步」里打开）：上传到网盘的备份用用户自己定的密码加密。
// 本机的一键导出不加密——别的输入法导出的词库、学习记录也是明文；导出时可以选要包含哪些数据。
//
// 做法是通行的口令加密，全部用 Go 标准库：PBKDF2-HMAC-SHA256 从密码导出密钥
// （迭代次数取 OWASP 对这个算法的建议值），AES-256-GCM 加密并校验内容没被改过。
// 密码只保存在本机（sync.json，和网盘密码一样用系统的 DPAPI 加密），不上传；
// 每台电脑要填同一个密码。忘了密码，加密的备份就打不开。
//
// 加密后的内容是一个 JSON 对象：
//
//	{"format":"freetype-encrypted-backup","version":1,"kdf":"pbkdf2-hmac-sha256",
//	 "iterations":600000,"salt":"…","cipher":"aes-256-gcm","nonce":"…","data":"…"}
const (
	sealedFormat     = "freetype-encrypted-backup"
	sealedKDF        = "pbkdf2-hmac-sha256"
	sealedCipher     = "aes-256-gcm"
	sealedIterations = 600_000
	// 打开备份时接受的迭代次数范围：太少不安全，太多会卡很久（文件来自网盘，可能被人改过）。
	sealedMinIterations = 100_000
	sealedMaxIterations = 10_000_000
)

var (
	errBackupPassword = errors.New("备份密码不正确，或者云端备份已损坏")
	errBackupLocked   = errors.New("云端备份已加密。请在「云端同步」里填写上传它的电脑用的备份密码")
	errNoPassword     = errors.New("已开启备份加密，请先填写并保存备份密码")
)

// sealedBackup 是加密后的内容。[]byte 字段在 JSON 里是 base64。
type sealedBackup struct {
	Format     string `json:"format"`
	Version    int    `json:"version"`
	KDF        string `json:"kdf"`
	Iterations int    `json:"iterations"`
	Salt       []byte `json:"salt"`
	Cipher     string `json:"cipher"`
	Nonce      []byte `json:"nonce"`
	Data       []byte `json:"data"`
}

// 文件头里的参数一起参加校验：改了算法、迭代次数的文件解不开，而不是按改过的参数去解。
func (b sealedBackup) associated() []byte {
	b.Salt, b.Nonce, b.Data = nil, nil, nil
	data, _ := json.Marshal(b)
	return data
}

func sealedKey(password string, salt []byte, iterations int) (cipher.AEAD, error) {
	key, err := pbkdf2.Key(sha256.New, password, salt, iterations, 32)
	if err != nil {
		return nil, err
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	return cipher.NewGCM(block)
}

// sealBackup 用密码加密一份备份，返回单行 JSON。
func sealBackup(plain []byte, password string) ([]byte, error) {
	if password == "" {
		return nil, errNoPassword
	}
	sealed := sealedBackup{Format: sealedFormat, Version: 1, KDF: sealedKDF, Iterations: sealedIterations,
		Salt: make([]byte, 16), Cipher: sealedCipher, Nonce: make([]byte, 12)}
	if _, err := io.ReadFull(rand.Reader, sealed.Salt); err != nil {
		return nil, err
	}
	if _, err := io.ReadFull(rand.Reader, sealed.Nonce); err != nil {
		return nil, err
	}
	aead, err := sealedKey(password, sealed.Salt, sealed.Iterations)
	if err != nil {
		return nil, err
	}
	sealed.Data = aead.Seal(nil, sealed.Nonce, plain, sealed.associated())
	return json.Marshal(sealed)
}

// openBackup 还原一份备份：没加密的原样返回；加密的用密码解开。
// 没有密码返回 errBackupLocked，密码不对或内容被改过返回 errBackupPassword。
func openBackup(data []byte, password string) ([]byte, error) {
	var sealed sealedBackup
	trimmed := bytes.TrimSpace(data)
	if len(trimmed) == 0 || trimmed[0] != '{' || json.Unmarshal(trimmed, &sealed) != nil || sealed.Format != sealedFormat {
		return data, nil
	}
	if sealed.Version != 1 || sealed.KDF != sealedKDF || sealed.Cipher != sealedCipher {
		return nil, errors.New("这份云端备份由更新的 FreeType 加密，请先更新这台电脑上的 FreeType")
	}
	if sealed.Iterations < sealedMinIterations || sealed.Iterations > sealedMaxIterations || len(sealed.Salt) < 8 || len(sealed.Nonce) != 12 {
		return nil, errors.New("云端备份的加密参数无效，文件可能已损坏")
	}
	if password == "" {
		return nil, errBackupLocked
	}
	aead, err := sealedKey(password, sealed.Salt, sealed.Iterations)
	if err != nil {
		return nil, err
	}
	plain, err := aead.Open(nil, sealed.Nonce, sealed.Data, sealed.associated())
	if err != nil {
		return nil, errBackupPassword
	}
	return plain, nil
}

// sealingStore 在远端存储外面加一层：上传的备份按密码加密，下载时解开。
//
// 只动同步根目录下的备份文件（sync-*.json）。首行的设备和版本信息保持明文——列出各台电脑的
// 备份时只读这一行；其余内容（设置、词库、短语、学习记录）整体加密。上传、下载两头的大小和
// 摘要校验照旧按明文算，所以别处的同步逻辑不用知道有没有加密。
type sealingStore struct {
	remoteStore
	seal string // 上传时加密用的密码；空表示不加密
	open string // 下载时解密用的密码；空表示遇到加密的备份就报错
}

// withBackupPassword 按同步设置给远端存储加上加密层。没开加密时上传明文；
// 填着的密码仍用来打开别的电脑加密上传的备份。
func withBackupPassword(store remoteStore, c syncSettings) remoteStore {
	layer := &sealingStore{remoteStore: store, open: c.EncryptPassword}
	if c.Encrypt {
		layer.seal = c.EncryptPassword
	}
	return layer
}

// 旧目录只读（见 syncDevice.snapshotStore）；里面是旧版的明文备份，原样读。
func (s *sealingStore) legacyStore() remoteStore {
	if source, ok := s.remoteStore.(interface{ legacyStore() remoteStore }); ok {
		return source.legacyStore()
	}
	return nil
}

func sealedSnapshotKey(key string) bool {
	return strings.HasPrefix(key, "sync-") && strings.HasSuffix(key, ".json") && !strings.Contains(key, "/")
}

// sealed 把一份要上传的备份换成加密的：首行（设备和版本信息）不动，后面的内容加密。
func (s *sealingStore) sealed(body io.ReadSeeker) (io.ReadSeeker, int64, error) {
	data, err := io.ReadAll(body)
	if err != nil {
		return nil, 0, err
	}
	// 首行是 {"sync_metadata":{…}, ；后面接着原来那个 JSON 对象去掉开头的 {
	end := bytes.IndexByte(data, '\n')
	if end < 0 {
		return nil, 0, errors.New("生成的同步备份格式不正确")
	}
	sealed, err := sealBackup(append([]byte{'{'}, data[end+1:]...), s.seal)
	if err != nil {
		return nil, 0, err
	}
	out := append(append([]byte{}, data[:end+1]...), sealed[1:]...)
	return bytes.NewReader(out), int64(len(out)), nil
}

func (s *sealingStore) Put(key string, body io.ReadSeeker, size int64) error {
	if s.seal != "" && sealedSnapshotKey(key) {
		var err error
		if body, size, err = s.sealed(body); err != nil {
			return err
		}
	}
	return s.remoteStore.Put(key, body, size)
}

func (s *sealingStore) PutNew(key string, body io.ReadSeeker, size int64) error {
	if s.seal != "" && sealedSnapshotKey(key) {
		var err error
		if body, size, err = s.sealed(body); err != nil {
			return err
		}
	}
	return s.remoteStore.PutNew(key, body, size)
}

// snapshotSplitter 把首行直接交给读的人（只要首行的可以马上停，见 syncHeaderReader），
// 其余部分先攒着，等整份下载完再解密。
type snapshotSplitter struct {
	out  io.Writer
	body bytes.Buffer
	head bool // 首行已经交出去了
}

func (w *snapshotSplitter) Write(p []byte) (int, error) {
	if w.head {
		return w.body.Write(p)
	}
	end := bytes.IndexByte(p, '\n')
	if end < 0 {
		return w.out.Write(p)
	}
	if n, err := w.out.Write(p[:end+1]); err != nil {
		return n, err
	}
	w.head = true
	w.body.Write(p[end+1:])
	return len(p), nil
}

func (s *sealingStore) Get(key string, out io.Writer) error {
	if !sealedSnapshotKey(key) {
		return s.remoteStore.Get(key, out)
	}
	split := &snapshotSplitter{out: out}
	if err := s.remoteStore.Get(key, split); err != nil {
		return err
	}
	if !split.head {
		return nil
	}
	plain, err := openBackup(append([]byte{'{'}, split.body.Bytes()...), s.open)
	if err != nil {
		return err
	}
	_, err = out.Write(plain[1:])
	return err
}
