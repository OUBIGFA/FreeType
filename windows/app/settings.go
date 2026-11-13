package main

import (
	"fmt"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

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
	// 选文件、存文件、打开文件夹（测试里换掉）。
	openFile   func(title string) string
	openTable  func(title string) string
	saveFile   func(title, name string) string
	openFolder func(path string)
	// 安装目录里的资源（图标），测试里为空。
	resources string
	// 托盘图标开关改了（托盘程序跟着建或拿掉图标）。
	onTray func(show bool)

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
	fonts []string

	dicts   []Dictionary
	phrases []QuickPhrase
	common  []string
	words   []UserWord

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

	// 最近一次导入的结果。
	report       string
	reportErrors []string
	// 下一帧弹出的提示。
	notice string
}

func newSettings(b Backend, version string) *settings {
	return &settings{
		backend: b,
		version: version,
		page:    "general",
		config:  defaultConfig(),
		apply:   func(fn func()) { fn() },
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
	s.background(func() func() {
		status, err := s.backend.Call(op("status"))
		if err != nil {
			return func() { s.statusErr = err.Error() }
		}
		config, cerr := s.backend.Call(op("get_config"))
		dicts, _ := s.backend.Call(op("list_dictionaries"))
		phrases, _ := s.backend.Call(op("list_quick_phrases"))
		common, commonErr := s.backend.Call(op("list_common_phrases"))
		return func() {
			s.statusErr = ""
			if status.Status != nil {
				s.status = *status.Status
			}
			if cerr == nil && config.Config != nil {
				s.config = *config.Config
				s.loaded = true
			}
			s.dicts = dicts.Dictionaries
			s.phrases = phrases.QuickPhrases
			if commonErr == nil {
				s.common = common.CommonPhrases
			} else {
				s.notice = "读取常用语失败：" + commonErr.Error()
			}
		}
	})
}

// saveConfig 把整份设置交给服务进程（立即生效）。
func (s *settings) saveConfig() {
	c := s.config
	if s.onTray != nil {
		s.onTray(c.TrayIcon)
	}
	s.background(func() func() {
		resp, err := s.backend.Call(op("set_config", "config", c))
		return func() {
			if err != nil {
				s.notice = "保存失败：" + err.Error()
				return
			}
			if resp.Config != nil {
				s.config = *resp.Config
			}
		}
	})
}

// run 发一条命令（切中英、标点、简繁），刷新状态。
func (s *settings) run(req Request) {
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
			if config.Config != nil {
				s.config = *config.Config
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
		msg += "（" + r.Note + "）"
	}
	return msg, r.Errors
}

func (s *settings) importDictionary() {
	s.background(func() func() {
		path := s.openFile("导入词库")
		if path == "" {
			return nil
		}
		name := strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
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
				s.notice = "已删除「" + name + "」"
			}
			s.dicts = list.Dictionaries
		}
	})
}

func (s *settings) export(request, title, name string) {
	s.background(func() func() {
		path := s.saveFile(title, name)
		if path == "" {
			return nil
		}
		resp, err := s.backend.Call(op(request, "path", path))
		return func() {
			if err != nil {
				s.notice = "导出失败：" + err.Error()
				return
			}
			count := 0
			if resp.Report != nil {
				count = resp.Report.Added
			}
			s.notice = fmt.Sprintf("已导出 %d 条", count)
			if resp.Report != nil && resp.Report.Note != "" {
				s.notice += "（" + resp.Report.Note + "）"
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
		resp, err := state.backend.Call(op("import_learning", "path", path))
		return func() {
			if err != nil {
				state.notice = "导入失败：" + err.Error()
				return
			}
			state.words = resp.UserWords
			state.wordsLoaded = true
			state.notice, _ = describe(resp.Report, "学习记录已合并")
		}
	})
}

func (s *settings) importPhrases() {
	replace := s.replacePhrases
	s.background(func() func() {
		path := s.openFile("导入快捷短语")
		if path == "" {
			return nil
		}
		resp, err := s.backend.Call(op("import_quick_phrases", "path", path, "replace", replace))
		list, _ := s.backend.Call(op("list_quick_phrases"))
		return func() {
			if err != nil {
				s.report, s.reportErrors = "导入失败："+err.Error(), nil
			} else {
				s.report, s.reportErrors = describe(resp.Report, "快捷短语新增")
			}
			s.phrases = list.QuickPhrases
		}
	})
}

// savePhrases 整份替换快捷短语。
func (s *settings) savePhrases(phrases []QuickPhrase, done string) {
	s.background(func() func() {
		_, err := s.backend.Call(op("set_quick_phrases", "phrases", phrases))
		list, _ := s.backend.Call(op("list_quick_phrases"))
		return func() {
			if err != nil {
				s.notice = "保存失败：" + err.Error()
			} else {
				s.notice = done
			}
			s.phrases = list.QuickPhrases
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
	code := strings.ToLower(strings.TrimSpace(s.newCode))
	text := strings.ReplaceAll(s.newText, "\r\n", "\n")
	weight := 10
	if w := strings.TrimSpace(s.newWeight); w != "" {
		n, err := strconv.Atoi(w)
		if err != nil || n < 0 {
			s.formError = "权重必须是非负整数。"
			return
		}
		weight = n
	}
	switch {
	case !validCode(code):
		s.formError = "编码只能包含英文字母。"
		return
	case strings.TrimSpace(text) == "":
		s.formError = "短语不能为空。"
		return
	}
	s.formError = ""
	phrases := append(append([]QuickPhrase(nil), s.phrases...), QuickPhrase{Code: code, Text: text, Weight: weight})
	s.newCode, s.newText, s.newWeight = "", "", ""
	s.savePhrases(phrases, "已添加「"+code+"」")
}

func (s *settings) removePhrase(i int) {
	if i < 0 || i >= len(s.phrases) {
		return
	}
	phrases := append(append([]QuickPhrase(nil), s.phrases[:i]...), s.phrases[i+1:]...)
	s.savePhrases(phrases, "已删除「"+s.phrases[i].Code+"」")
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
				s.notice = "已删除「" + words[0] + "」的记忆"
			} else {
				s.notice = fmt.Sprintf("已删除 %d 个词的记忆", len(words))
			}
		}
	})
}

// addWord 自己加一个词；拼音可以空着，服务进程按常用读音注音。
func (s *settings) addWord() {
	word, pinyin := strings.TrimSpace(s.newWord), strings.TrimSpace(s.newPinyin)
	if word == "" {
		s.wordError = "请填写词语。"
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
			s.notice = "已清空学到的词和输入习惯"
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
		pinyin := strings.ReplaceAll(w.Pinyin, " ", "")
		if strings.Contains(w.Word, q) || strings.Contains(pinyin, compact) {
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
