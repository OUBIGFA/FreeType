package main

import (
	"fmt"
	"path/filepath"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/egoist/mygo/ui"
)

// settings 是设置窗口的状态。界面在主线程上画；找服务进程的事放到 goroutine 里做，
// 做完用 apply 回到主线程改状态。
type settings struct {
	backend Backend
	version string
	// apply 在主线程上执行 fn 并重画；测试里直接执行。
	apply func(fn func())
	// inline：后台的事也在当前线程做完（测试用）。
	inline bool
	// 保存请求按操作顺序发送；旧应答不能覆盖用户随后改过的界面值。
	configSaveTail <-chan struct{}
	configRevision uint64
	// 启动结束与再次打开窗口都可刷新；晚到的旧快照不能覆盖较新的结果。
	refreshRevision uint64
	// 保存失败时恢复已确认的图标，避免预览与实际托盘显示不同的外观。
	savedBrandIcon, savedCustomBrandIcon string
	// 选文件、存文件、打开文件夹（测试里换掉）。
	openFile                                          func(title string) string
	openTable                                         func(title string) string
	openModel                                         func(title string) string
	defaultTip                                        func(change bool) (bool, error)
	defaultTipBusy, defaultTipKnown, defaultTipActive bool
	modelBusy, confirmModelReset                      bool
	saveFile                                          func(title, name string) string
	openFolder                                        func(path string)
	openURL                                           func(url string) error
	// 安装目录里的资源（图标），测试里为空。
	resources string
	// 托盘图标开关改了（托盘程序跟着建或拿掉图标）。
	onTray func(show bool)
	// 品牌图标在主线程热更新；文件选择与图像解码由导入动作完成。
	onBrandIcon   func(Config) error
	openIcon      func() (string, error)
	brandIconBusy bool
	// 更新动作由应用接入真实更新库；视图只显示状态并提交用户操作。
	onCheckUpdates    func()
	onUpdateNow       func()
	onAutomaticUpdate func(bool)
	updateStatus      updateState

	page                         string
	loaded                       bool
	status                       Status
	statusErr                    string
	config                       Config
	settingQuery, locatedSetting string
	revealSetting                bool
	pageScroll, searchScroll     ui.ScrollState
	locatedHandle                ui.Handle

	// 字体列表（第一次打开候选窗页时取）。
	fonts            []string
	shuangpinExample string
	newVimApp        string

	dicts                                                                    []Dictionary
	dictionaryName, dictionaryQuery, dictionaryRename, dictionaryDescription string
	dictionaryPage                                                           *DictionaryPage
	dictionarySequence                                                       uint64
	dictionaryBusy, confirmDictionaryDelete                                  bool
	dictionarySelected                                                       map[string]bool
	phrases                                                                  []QuickPhrase
	selectedPhrases, selectedWords                                           map[string]bool
	confirmPhraseBatch, confirmWordBatch                                     bool
	common                                                                   []string
	// 全列表保存完成并刷新前不接受下一次修改，避免用旧列表覆盖刚写入的条目。
	phrasesBusy, commonBusy      bool
	words                        []UserWord
	stats                        map[string]typingDay
	statsBusy, confirmStatsClear bool
	statsError                   string
	statsTab, statsScope         int
	statsUpdatedAt               time.Time

	// 最近一次从服务端发现的可选窗口。
	apps                                     []AppWindow
	newPresetApp, newPresetMode, presetError string

	newCode, newText, newWeight         string
	formError                           string
	replacePhrases                      bool
	phraseQuery                         string
	newCommon, commonQuery, commonError string
	replaceCommon                       bool

	wordQuery                   string
	newWord                     string
	newPinyin                   string
	wordError                   string
	wordsLoaded                 bool
	confirmClear                bool
	confirmWipe                 bool
	confirmRestore              bool
	restorePath, restoreSummary string
	// 一键导出：正在选要导出的数据；exportParts 和 backupParts 一一对应（空：全选）。
	exportChoosing                    bool
	exportParts                       []bool
	exportPassword                    string
	unlockingRestore                  bool
	lockedBackupPath, restorePassword string
	unlockError                       string
	unlockLearning                    bool

	// 数据同步（见 sync.go）。sync 是 sync.json 的内容，第一次打开那一页时读。
	sync           syncSettings
	syncLoaded     bool
	autoSync       *autoSync
	autoSyncStatus autoSyncState
	// 同步设置放在哪个目录；空着用服务进程报的用户数据目录（测试里换掉）。
	syncDir string
	// 连远端的办法（测试里换成内存里的假远端）。
	newStore func(syncSettings) (remoteStore, error)
	// 正在做的同步动作的说明（空：没有在做）；做完的结果和它是不是失败。
	syncBusy, syncMessage             string
	syncMessageArea                   string
	showSyncHelp                      bool
	syncFailed                        bool
	syncDevices                       []syncDevice
	syncListed                        bool
	syncRestore                       syncDevice
	syncPreview                       *SyncMergePreview
	syncPreviewFile, syncExpectedFile string
	syncReplace                       bool

	// 最近一次导入的结果。
	report       string
	reportErrors []string
	// 下一帧弹出的提示。
	notice string
}

func newSettings(b Backend, version string) *settings {
	config := defaultConfig()
	return &settings{
		backend:        b,
		version:        version,
		page:           "general",
		config:         config,
		savedBrandIcon: config.BrandIcon,
		apply:          func(fn func()) { fn() },
	}
}

// background 在后台做 work，做完把它返回的改动交给主线程。
func (s *settings) background(work func() func()) {
	if s.inline {
		if change := work(); change != nil {
			change()
		}
		return
	}
	go func() {
		if change := work(); change != nil {
			s.apply(change)
		}
	}()
}

// refresh 重新读状态、设置、词库和快捷短语。
func (s *settings) refresh() {
	s.refreshDefaultTip(false)
	s.refreshRevision++
	refreshRevision := s.refreshRevision
	revision := s.configRevision
	s.background(func() func() {
		status, err := s.backend.Call(op("status"))
		if err != nil {
			return func() {
				if refreshRevision == s.refreshRevision {
					s.statusErr = err.Error()
				}
			}
		}
		config, cerr := s.backend.Call(op("get_config"))
		dicts, dictErr := s.backend.Call(op("list_dictionaries"))
		phrases, phraseErr := s.backend.Call(op("list_quick_phrases"))
		common, commonErr := s.backend.Call(op("list_common_phrases"))
		apps, _ := s.backend.Call(op("list_apps"))
		return func() {
			if refreshRevision != s.refreshRevision {
				return
			}
			s.statusErr = ""
			if status.Status != nil {
				s.status = *status.Status
			}
			if cerr == nil && config.Config != nil && revision == s.configRevision {
				s.config = *config.Config
				s.shuangpinExample = config.ShuangpinExample
				s.loaded = true
				s.updateBrandIcon()
			}
			if cerr != nil && revision == s.configRevision {
				s.notice = "读取设置失败：" + cerr.Error()
			}
			if dictErr == nil {
				s.dicts = dicts.Dictionaries
			} else {
				s.notice = "读取词库列表失败：" + dictErr.Error()
			}
			if phraseErr == nil {
				s.phrases = phrases.QuickPhrases
			} else {
				s.notice = "读取快捷短语失败：" + phraseErr.Error()
			}
			if commonErr == nil {
				s.common = common.CommonPhrases
			} else {
				s.notice = "读取常用语失败：" + commonErr.Error()
			}
			s.apps = apps.Apps
		}
	})
}

// saveConfig 把整份设置交给服务进程（立即生效）。
func (s *settings) saveConfig() {
	c := s.config
	c.AppPresetRules = slices.Clone(c.AppPresetRules)
	s.configRevision++
	revision := s.configRevision
	previous := s.configSaveTail
	done := make(chan struct{})
	s.configSaveTail = done
	if s.onTray != nil {
		s.onTray(c.TrayIcon)
	}
	s.background(func() func() {
		if previous != nil {
			<-previous
		}
		resp, err := s.backend.Call(op("set_config", "config", c))
		close(done)
		return func() {
			if err == nil {
				confirmed := c
				if resp.Config != nil {
					confirmed = *resp.Config
				}
				s.savedBrandIcon, s.savedCustomBrandIcon = confirmed.BrandIcon, confirmed.CustomBrandIcon
			}
			if revision != s.configRevision {
				return
			}
			if err != nil {
				s.config.BrandIcon, s.config.CustomBrandIcon = s.savedBrandIcon, s.savedCustomBrandIcon
				s.updateBrandIcon()
				s.notice = "保存失败：" + err.Error()
				return
			}
			if resp.Config != nil {
				s.config = *resp.Config
				s.shuangpinExample = resp.ShuangpinExample
			}
			s.updateBrandIcon()
			if s.onAutomaticUpdate != nil {
				s.onAutomaticUpdate(s.config.AutoUpdate)
			}
			s.wordsLoaded = false
		}
	})
}

// run 发一条命令（切中英、标点、简繁），刷新状态。
func (s *settings) run(req Request) {
	revision := s.configRevision
	s.background(func() func() {
		_, err := s.backend.Call(req)
		status, serr := s.backend.Call(op("status"))
		config, _ := s.backend.Call(op("get_config"))
		return func() {
			if err != nil {
				s.notice = "操作失败：" + err.Error()
				return
			}
			if serr == nil && status.Status != nil {
				s.status = *status.Status
			}
			if config.Config != nil && revision == s.configRevision {
				s.config = *config.Config
				s.shuangpinExample = config.ShuangpinExample
				s.updateBrandIcon()
			}
		}
	})
}

func describe(r *ImportReport, what string) (string, []string) {
	if r == nil {
		return what + "完成", nil
	}
	msg := fmt.Sprintf("%s %d 条", what, r.Added)
	if r.Skipped > 0 {
		msg += fmt.Sprintf("，跳过 %d 行", r.Skipped)
	}
	if r.Note != "" {
		msg += "；" + r.Note
	}
	return msg, r.Errors
}

// dictionaryName 是导入后显示的词库名：文件名去掉扩展名；Rime 词典 rime_ice.dict.yaml 叫 rime_ice。
func dictionaryName(path string) string {
	name := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
	if ext := strings.ToLower(filepath.Ext(path)); ext == ".yaml" || ext == ".yml" {
		name = strings.TrimSuffix(name, ".dict")
	}
	return name
}

func (s *settings) importDictionary() {
	s.background(func() func() {
		path := s.openFile("导入词库")
		if path == "" {
			return nil
		}
		name := dictionaryName(path)
		resp, err := s.backend.Call(op("import_dictionary", "path", path, "name", name))
		list, _ := s.backend.Call(op("list_dictionaries"))
		return func() {
			if err != nil {
				s.report, s.reportErrors = "导入失败："+err.Error(), nil
			} else {
				s.report, s.reportErrors = describe(resp.Report, "「"+name+"」导入")
			}
			s.dicts = list.Dictionaries
		}
	})
}

// chooseLexicon 换主词库文件（空串恢复默认）；服务进程在后台重新载入。
func (s *settings) chooseLexicon(path string) {
	s.config.Lexicon = path
	s.saveConfig()
	s.notice = "正在载入新的词库，几秒后生效"
	s.background(func() func() {
		status, err := s.backend.Call(op("status"))
		return func() {
			if err == nil && status.Status != nil {
				s.status = *status.Status
			}
		}
	})
}

func (s *settings) removeDictionary(name string) {
	s.background(func() func() {
		_, err := s.backend.Call(op("remove_dictionary", "name", name))
		list, _ := s.backend.Call(op("list_dictionaries"))
		return func() {
			if err != nil {
				s.notice = "删除失败：" + err.Error()
			} else {
				s.notice = "已删除 " + name
			}
			s.dicts = list.Dictionaries
		}
	})
}

func (s *settings) export(request, title, name string) {
	// 选了哪些数据在界面线程里取好，后台只用这份副本
	parts := s.chosenExportParts()
	password := s.exportPassword
	s.exportPassword = ""
	s.background(func() func() {
		path := s.saveFile(title, name)
		if path == "" {
			return nil
		}
		var resp Response
		var err error
		if request == "export_settings" {
			resp, err = s.exportSettingsBackup(path, parts, password)
		} else {
			resp, err = s.backend.Call(op(request, "path", path))
		}
		return func() {
			if err != nil {
				s.notice = "导出失败：" + err.Error()
				return
			}
			count := 0
			if resp.Report != nil {
				count = resp.Report.Added
			}
			if request != "export_settings" {
				s.notice = fmt.Sprintf("已导出 %d 条", count)
				if resp.Report != nil && resp.Report.Note != "" {
					s.notice += "；" + resp.Report.Note
				}
			} else if len(parts) == len(backupParts) {
				s.notice = "已导出全部设置和数据"
			} else {
				s.notice = "已导出选中的数据：" + exportedSummary(resp, parts)
			}
			if request == "export_settings" && password != "" {
				s.notice += "；已加密"
			}
		}
	})
}

func (state *settings) importLearning() {
	state.background(func() func() {
		path := state.openFile("导入学习备份")
		if path == "" {
			return nil
		}
		if locked, _ := checkEncryptedBackupFile(path); locked {
			return func() {
				state.lockedBackupPath = path
				state.restorePassword = ""
				state.unlockError = ""
				state.unlockLearning = true
				state.unlockingRestore = true
			}
		}
		return state.importLearningFromFile(path, "")
	})
}

func (s *settings) importPhrases() {
	if s.phrasesBusy {
		return
	}
	s.phrasesBusy = true
	replace := s.replacePhrases
	s.background(func() func() {
		path := s.openFile("导入快捷短语")
		if path == "" {
			return func() { s.phrasesBusy = false }
		}
		resp, err := s.backend.Call(op("import_quick_phrases", "path", path, "replace", replace))
		list, listErr := s.backend.Call(op("list_quick_phrases"))
		return func() {
			s.phrasesBusy = false
			if err != nil {
				s.report, s.reportErrors = "导入失败："+err.Error(), nil
			} else {
				s.report, s.reportErrors = describe(resp.Report, "快捷短语新增")
			}
			if listErr == nil {
				s.phrases = list.QuickPhrases
			} else {
				s.notice = "刷新快捷短语失败：" + listErr.Error()
			}
		}
	})
}

// savePhrases 整份替换快捷短语。
func (s *settings) savePhrases(phrases []QuickPhrase, done func()) {
	if s.phrasesBusy {
		return
	}
	s.phrasesBusy = true
	// Rust 协议把 null 视为格式错误；删除最后一条也必须发送列表。
	if phrases == nil {
		phrases = []QuickPhrase{}
	}
	s.background(func() func() {
		_, err := s.backend.Call(op("set_quick_phrases", "phrases", phrases))
		return func() {
			s.phrasesBusy = false
			if err != nil {
				s.notice = "保存失败：" + err.Error()
			} else {
				// 保存请求已完整验证；直接接纳这份列表，避免后续刷新失败丢失界面状态。
				s.phrases = phrases
				done()
			}
		}
	})
}

// validCode 判断快捷短语编码：只能是英文字母。
func validCode(code string) bool {
	if code == "" {
		return false
	}
	for _, r := range code {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z') {
			return false
		}
	}
	return true
}

func (s *settings) addPhrase() {
	if s.phrasesBusy {
		return
	}
	inputCode, inputText, inputWeight := s.newCode, s.newText, s.newWeight
	code := strings.ToLower(strings.TrimSpace(s.newCode))
	text := strings.ReplaceAll(s.newText, "\r\n", "\n")
	weight := 10
	if w := strings.TrimSpace(s.newWeight); w != "" {
		n, err := strconv.Atoi(w)
		if err != nil || n < 0 {
			s.formError = "权重必须是 0 或正整数"
			return
		}
		weight = n
	}
	switch {
	case !validCode(code):
		s.formError = "编码只能包含英文字母"
		return
	case strings.TrimSpace(text) == "":
		s.formError = "短语不能为空"
		return
	}
	s.formError = ""
	phrases := append(append([]QuickPhrase(nil), s.phrases...), QuickPhrase{Code: code, Text: text, Weight: weight})
	s.savePhrases(phrases, func() {
		if s.newCode == inputCode && s.newText == inputText && s.newWeight == inputWeight {
			s.newCode, s.newText, s.newWeight = "", "", ""
		}
		s.notice = "已添加 " + code
	})
}

func (s *settings) removePhrase(i int) {
	if i < 0 || i >= len(s.phrases) {
		return
	}
	phrases := append(append([]QuickPhrase(nil), s.phrases[:i]...), s.phrases[i+1:]...)
	code := s.phrases[i].Code
	s.savePhrases(phrases, func() { s.notice = "已删除 " + code })
}

// loadWords 读学到的词。
func (s *settings) loadWords() {
	s.background(func() func() {
		resp, err := s.backend.Call(op("list_user_words"))
		return func() {
			s.wordsLoaded = true
			if err != nil {
				s.notice = "读取失败：" + err.Error()
				return
			}
			s.words = resp.UserWords
		}
	})
}

// forgetWords 忘掉这些学到的词。
func (s *settings) forgetWords(words []string) {
	s.background(func() func() {
		resp, err := s.backend.Call(op("forget_words", "words", words))
		return func() {
			if err != nil {
				s.notice = "删除失败：" + err.Error()
				return
			}
			s.words = resp.UserWords
			if len(words) == 1 {
				s.notice = "已删除 " + words[0] + " 的学习记录"
			} else {
				s.notice = fmt.Sprintf("已删除 %d 个词的学习记录", len(words))
			}
		}
	})
}

// addWord 自己加一个词；拼音可以空着，服务进程按常用读音注音。
func (s *settings) addWord() {
	word, pinyin := strings.TrimSpace(s.newWord), strings.TrimSpace(s.newPinyin)
	if word == "" {
		s.wordError = "请填写词语"
		return
	}
	s.background(func() func() {
		resp, err := s.backend.Call(op("add_user_word", "word", word, "pinyin", pinyin))
		return func() {
			if err != nil {
				s.wordError = err.Error()
				return
			}
			s.wordError = ""
			s.newWord, s.newPinyin = "", ""
			s.words = resp.UserWords
			s.wordsLoaded = true
			added := word
			if resp.Report != nil && resp.Report.Note != "" {
				added = resp.Report.Note
			}
			s.notice = "已添加 " + added
		}
	})
}

func (s *settings) clearLearned() {
	s.background(func() func() {
		_, err := s.backend.Call(op("clear_learned"))
		return func() {
			if err != nil {
				s.notice = "清空失败：" + err.Error()
				return
			}
			s.words = nil
			s.notice = "已清空学习记录"
		}
	})
}

func (s *settings) clearClipboard() {
	s.background(func() func() {
		_, err := s.backend.Call(op("clear_clipboard"))
		return func() {
			if err != nil {
				s.notice = "清空失败：" + err.Error()
				return
			}
			s.notice = "已清空剪贴板历史"
		}
	})
}

// filteredWords 是按搜索词筛过的学到的词（词或拼音里含有它）。
func (s *settings) filteredWords() []UserWord {
	q := strings.ToLower(strings.TrimSpace(s.wordQuery))
	if q == "" {
		return s.words
	}
	var out []UserWord
	compact := strings.ReplaceAll(q, " ", "")
	for _, w := range s.words {
		pinyin := strings.ReplaceAll(strings.ToLower(w.Pinyin), " ", "")
		if strings.Contains(strings.ToLower(w.Word), q) || strings.Contains(strings.ToLower(w.text()), q) || strings.Contains(pinyin, compact) {
			out = append(out, w)
		}
	}
	return out
}

// filteredPhrases 是按搜索词筛过的快捷短语（返回原来的序号）。
func (s *settings) filteredPhrases() []int {
	q := strings.ToLower(strings.TrimSpace(s.phraseQuery))
	var out []int
	for i, p := range s.phrases {
		if q == "" || strings.Contains(p.Code, q) || strings.Contains(strings.ToLower(p.Text), q) {
			out = append(out, i)
		}
	}
	return out
}

// fontList 是能选的字体（常用中文字体在前）。
func (s *settings) fontList(all func() []string) []string {
	if s.fonts == nil {
		preferred := []string{"Microsoft YaHei", "Microsoft YaHei UI", "DengXian", "SimSun", "SimHei", "KaiTi", "FangSong", "Source Han Sans SC", "Noto Sans SC", "HarmonyOS Sans SC", "MiSans", "LXGW WenKai"}
		have := map[string]bool{}
		for _, f := range all() {
			have[f] = true
		}
		var out []string
		seen := map[string]bool{}
		for _, f := range preferred {
			if have[f] || f == "Microsoft YaHei" {
				out = append(out, f)
				seen[f] = true
			}
		}
		var rest []string
		for f := range have {
			if !seen[f] && !strings.HasPrefix(f, "@") {
				rest = append(rest, f)
			}
		}
		sort.Strings(rest)
		s.fonts = append(out, rest...)
	}
	return s.fonts
}

// preview 是短语在列表里的样子：只显示第一行，后面还有行时加 ↵。
func preview(text string) string {
	first, _, more := strings.Cut(text, "\n")
	if more {
		return first + " ↵"
	}
	return first
}
