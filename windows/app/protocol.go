package main

// 服务进程管道消息（与 windows/protocol/src/lib.rs 一致）：每条请求、应答各一行 JSON。

// Config 是用户设置，字段名与服务进程的 config.json 相同。
// 必须列全：保存时整份发回服务进程，少一个字段那一项就回到默认值。
type Config struct {
	PageSize   int    `json:"page_size"`
	Horizontal bool   `json:"horizontal"`
	FontSize   int    `json:"font_size"`
	FontFamily string `json:"font_family"`

	Fuzzy           bool   `json:"fuzzy"`
	FuzzyGroups     int    `json:"fuzzy_groups"`
	Shuangpin       bool   `json:"shuangpin"`
	ShuangpinScheme string `json:"shuangpin_scheme"`
	TypoCorrection  bool   `json:"typo_correction"`
	RareChars       bool   `json:"rare_chars"`
	Lexicon         string `json:"lexicon"`

	ShiftToggle string `json:"shift_toggle"`
	PageKeys    int    `json:"page_keys"`
	SelectKeys  string `json:"select_keys"`
	PickChar    bool   `json:"pick_char"`
	NumpadDot   bool   `json:"numpad_dot"`

	FullwidthPunct   bool   `json:"fullwidth_punct"`
	AutoPair         bool   `json:"auto_pair"`
	Autospace        bool   `json:"autospace"`
	AutospaceLetters bool   `json:"autospace_letters"`
	AutospaceDigits  bool   `json:"autospace_digits"`
	DefaultMode      string `json:"default_mode"`
	ModeScope        string `json:"mode_scope"`
	Traditional      bool   `json:"traditional"`

	QuickPhrases     bool `json:"quick_phrases"`
	EmojiSuggest     bool `json:"emoji_suggest"`
	DateTime         bool `json:"date_time"`
	Calculator       bool `json:"calculator"`
	Toolbar          bool `json:"toolbar"`
	ClipboardHistory bool `json:"clipboard_history"`
	EmailCompletion  bool `json:"email_completion"`

	HotkeyEmoji       string `json:"hotkey_emoji"`
	HotkeyPunct       string `json:"hotkey_punct"`
	HotkeyTraditional string `json:"hotkey_traditional"`
	HotkeyMode        string `json:"hotkey_mode"`

	StatusBar         bool   `json:"status_bar"`
	StatusBarAutohide bool   `json:"status_bar_autohide"`
	HintOnToggle      bool   `json:"hint_on_toggle"`
	HintOnFocus       bool   `json:"hint_on_focus"`
	HintPosition      string `json:"hint_position"`
}

// defaultConfig 和服务进程的默认值一样（连不上服务进程时界面先按它画）。
func defaultConfig() Config {
	return Config{
		PageSize: 7, Horizontal: true, FontSize: 18, FontFamily: "Microsoft YaHei",
		FuzzyGroups: fuzzyDefault, ShuangpinScheme: "xiaohe", TypoCorrection: true,
		ShiftToggle: "both", PageKeys: pageKeyMinus, SelectKeys: "off", PickChar: true, NumpadDot: true,
		FullwidthPunct: true, AutoPair: true, Autospace: true, AutospaceLetters: true, AutospaceDigits: true,
		DefaultMode: "chinese", ModeScope: "global",
		QuickPhrases: true, EmojiSuggest: true, DateTime: true, Calculator: true, Toolbar: true, EmailCompletion: true,
		HotkeyEmoji: "off", HotkeyPunct: "off", HotkeyTraditional: "off", HotkeyMode: "off",
		HintOnToggle: true, HintPosition: "top-left",
	}
}

// 翻页键位（和服务进程一致）。
const (
	pageKeyMinus   = 1 << 0
	pageKeyBracket = 1 << 1
	pageKeyComma   = 1 << 2
	pageKeyTab     = 1 << 3
)

// 模糊音分组（和服务进程一致）。
const fuzzyDefault = 1 | 2 | 4 | 64 | 128 | 256 | 512 | 1024

var fuzzyRules = []struct {
	Bit   int
	Label string
}{
	{1, "z = zh"}, {2, "c = ch"}, {4, "s = sh"}, {8, "n = l"}, {16, "f = h"}, {32, "r = l"},
	{64, "an = ang"}, {128, "en = eng"}, {256, "in = ing"}, {512, "ian = iang"}, {1024, "uan = uang"}, {2048, "un = ong"},
}

// QuickPhrase 是一条快捷短语。
type QuickPhrase struct {
	Code   string `json:"code"`
	Text   string `json:"text"`
	Weight int    `json:"weight"`
}

// Dictionary 是一份导入的词库。
type Dictionary struct {
	Name    string `json:"name"`
	Entries int    `json:"entries"`
}

// UserWord 是一个学到的词。
type UserWord struct {
	Word   string  `json:"word"`
	Pinyin string  `json:"pinyin"`
	Freq   float64 `json:"freq"`
}

// Status 是服务进程的状态。
type Status struct {
	Protocol     int    `json:"protocol"`
	Version      string `json:"version"`
	Ready        bool   `json:"ready"`
	Mode         string `json:"mode"`
	ChinesePunct bool   `json:"chinese_punct"`
	Traditional  bool   `json:"traditional"`
	DataDir      string `json:"data_dir"`
	LogDir       string `json:"log_dir"`
	Lexicon      string `json:"lexicon"`
	Error        string `json:"error,omitempty"`
}

// ImportReport 是导入、导出的结果。
type ImportReport struct {
	Added   int      `json:"added"`
	Skipped int      `json:"skipped"`
	Errors  []string `json:"errors"`
	Note    string   `json:"note,omitempty"`
}

// Frame 只取设置程序用到的字段。
type Frame struct {
	Mode         string `json:"mode"`
	ChinesePunct bool   `json:"chinese_punct"`
	Traditional  bool   `json:"traditional"`
}

// Response 是一条应答。
type Response struct {
	OK           bool          `json:"ok"`
	Error        string        `json:"error,omitempty"`
	Status       *Status       `json:"status,omitempty"`
	Config       *Config       `json:"config,omitempty"`
	QuickPhrases []QuickPhrase `json:"quick_phrases,omitempty"`
	Dictionaries []Dictionary  `json:"dictionaries,omitempty"`
	UserWords    []UserWord    `json:"user_words,omitempty"`
	Report       *ImportReport `json:"report,omitempty"`
	Frame        *Frame        `json:"frame,omitempty"`
}

// Request 是一条请求：op 加上各自的字段。
type Request map[string]any

func op(name string, fields ...any) Request {
	r := Request{"op": name}
	for i := 0; i+1 < len(fields); i += 2 {
		r[fields[i].(string)] = fields[i+1]
	}
	return r
}

// command 是 DLL、托盘、状态栏都用的命令（切标点、简繁、打开设置……）。
func command(name string) Request {
	return op("command", "command", name)
}

// Backend 发请求、收应答；测试里换成假的。
type Backend interface {
	Call(Request) (Response, error)
}

// choice 是下拉框里的一项：存盘的写法和给人看的名字。
type choice struct{ ID, Name string }

var (
	schemes = []choice{
		{"xiaohe", "小鹤双拼"}, {"ziranma", "自然码"}, {"microsoft", "微软双拼"}, {"sogou", "搜狗双拼"},
		{"abc", "智能 ABC"}, {"ziguang", "紫光双拼"}, {"pinyinjiajia", "拼音加加"},
	}
	shiftToggles = []choice{{"both", "左右 Shift 都可以"}, {"left", "只用左 Shift"}, {"right", "只用右 Shift"}, {"off", "不用 Shift 切换"}}
	selectKeys   = []choice{{"off", "不用"}, {"semicolon", "分号选第 2 个"}, {"quote", "分号、引号选第 2、3 个"}}
	modes        = []choice{{"chinese", "中文"}, {"english", "英文"}}
	modeScopes   = []choice{{"global", "所有应用一致"}, {"app", "每个应用分别记住"}}
	positions    = []choice{
		{"top-left", "左上方"}, {"top", "正上方"}, {"top-right", "右上方"},
		{"bottom-left", "左下方"}, {"bottom", "正下方"}, {"bottom-right", "右下方"},
	}
	hotkeysEmoji       = []choice{{"off", "不用"}, {"ctrl+shift+e", "Ctrl + Shift + E"}, {"ctrl+alt+e", "Ctrl + Alt + E"}}
	hotkeysPunct       = []choice{{"off", "不用"}, {"ctrl+period", "Ctrl + ."}}
	hotkeysTraditional = []choice{{"off", "不用"}, {"ctrl+shift+f", "Ctrl + Shift + F"}, {"ctrl+shift+t", "Ctrl + Shift + T"}}
	hotkeysMode        = []choice{{"off", "不用"}, {"ctrl+space", "Ctrl + 空格"}}
	pageSizes          = []string{"3", "4", "5", "6", "7", "8", "9", "10"}
)

func names(list []choice) []string {
	out := make([]string, len(list))
	for i, c := range list {
		out[i] = c.Name
	}
	return out
}

func nameOf(list []choice, id string) string {
	for _, c := range list {
		if c.ID == id {
			return c.Name
		}
	}
	return list[0].Name
}

func idOf(list []choice, name string) string {
	for _, c := range list {
		if c.Name == name {
			return c.ID
		}
	}
	return list[0].ID
}
