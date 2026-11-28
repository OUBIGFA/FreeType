package main

// 服务进程管道消息（与 windows/protocol/src/lib.rs 一致）：每条请求、应答各一行 JSON。

// CandidateSkin 是自定义候选窗皮肤颜色（十六进制 #rrggbb）。
type CandidateSkin struct {
	Background string `json:"background"`
	Border     string `json:"border"`
	Text       string `json:"text"`
	Muted      string `json:"muted"`
	Accent     string `json:"accent"`
	AccentText string `json:"accent_text"`
	AccentFill bool   `json:"accent_fill"`
}

func defaultCandidateSkin() CandidateSkin {
	return CandidateSkin{
		Background: "#ffffff",
		Border:     "#d6d6d6",
		Text:       "#1f1f1f",
		Muted:      "#616161",
		Accent:     "#2b6ce6",
		AccentText: "#ffffff",
		AccentFill: true,
	}
}

// Config 是用户设置，字段名与服务进程的 config.json 相同。
// 必须列全：保存时整份发回服务进程，少一个字段那一项就回到默认值。
type Config struct {
	PageSize   int    `json:"page_size"`
	Horizontal bool   `json:"horizontal"`
	FontSize   int    `json:"font_size"`
	FontFamily string `json:"font_family"`
	// CandidateStyle 是选中候选的样式：pink blue green sogou custom。
	CandidateStyle string        `json:"candidate_style"`
	CandidateSkin  CandidateSkin `json:"candidate_skin"`
	// 品牌图标更新窗口、托盘和系统输入指示器；自定义 PNG 随设置保存。
	BrandIcon              string `json:"brand_icon"`
	CustomBrandIcon        string `json:"custom_brand_icon,omitempty"`
	CandidateWheel         bool   `json:"candidate_wheel"`
	InlinePreedit          string `json:"inline_preedit"`
	CandidatePreedit       bool   `json:"candidate_preedit"`
	SentenceEditor         bool   `json:"sentence_editor"`
	SentenceEditorPosition string `json:"sentence_editor_position"`
	PreeditSize            int    `json:"preedit_size"`
	CandidateFollow        bool   `json:"candidate_follow"`
	CandidateSingleLine    bool   `json:"candidate_single_line"`
	FontFallbacks          string `json:"font_fallbacks"`

	Fuzzy             bool   `json:"fuzzy"`
	FuzzyGroups       int    `json:"fuzzy_groups"`
	Shuangpin         bool   `json:"shuangpin"`
	ShuangpinScheme   string `json:"shuangpin_scheme"`
	ShuangpinShowKeys bool   `json:"shuangpin_show_keys"`
	TypoCorrection    bool   `json:"typo_correction"`
	TypoKinds         int    `json:"typo_kinds"`
	TypoMark          bool   `json:"typo_mark"`
	PinBadge          string `json:"pin_badge"`
	RareChars         bool   `json:"rare_chars"`
	Lexicon           string `json:"lexicon"`

	ShiftToggle string `json:"shift_toggle"`
	PageKeys    int    `json:"page_keys"`
	SelectKeys  string `json:"select_keys"`
	PickChar    bool   `json:"pick_char"`
	ArrowKeys   string `json:"arrow_keys"`
	NumpadDot   bool   `json:"numpad_dot"`

	FullwidthPunct       bool        `json:"fullwidth_punct"`
	DigitHalfwidthPunct  bool        `json:"digit_halfwidth_punct"`
	DigitPunctExceptions bool        `json:"digit_punct_exceptions"`
	DigitPunctFullwidth  int         `json:"digit_punct_fullwidth"`
	CornerQuotes         bool        `json:"corner_quotes"`
	AutoPair             bool        `json:"auto_pair"`
	AutoPairAbsorb       bool        `json:"auto_pair_absorb_closing"`
	DictionaryAutoUpdate bool        `json:"dictionary_auto_update"`
	Autospace            bool        `json:"autospace"`
	AutospaceLetters     bool        `json:"autospace_letters"`
	AutospaceDigits      bool        `json:"autospace_digits"`
	DefaultMode          string      `json:"default_mode"`
	ModeScope            string      `json:"mode_scope"`
	Traditional          bool        `json:"traditional"`
	AppPresets           bool        `json:"app_presets"`
	AppPresetRules       []AppPreset `json:"app_preset_rules"`
	VimModeApps          []string    `json:"vim_mode_apps"`
	EnglishCorrection    bool        `json:"english_correction"`
	EnglishSpace         bool        `json:"english_space"`
	EnglishGloss         bool        `json:"english_gloss"`
	HotkeyEnglish        string      `json:"hotkey_english"`
	HotkeyRadical        string      `json:"hotkey_radical"`

	QuickPhrases         bool `json:"quick_phrases"`
	EmojiSuggest         bool `json:"emoji_suggest"`
	DateTime             bool `json:"date_time"`
	Calculator           bool `json:"calculator"`
	Toolbar              bool `json:"toolbar"`
	ClipboardHistory     bool `json:"clipboard_history"`
	EmailCompletion      bool `json:"email_completion"`
	LongWordCompletion   bool `json:"long_word_completion"`
	PredictNext          bool `json:"predict_next"`
	PredictDigits        bool `json:"predict_digits"`
	PredictSpace         bool `json:"predict_space"`
	PredictDoubleSpace   bool `json:"predict_double_space"`
	PredictArrows        bool `json:"predict_arrows"`
	TypingStats          bool `json:"typing_stats"`
	CandidateDiagnostics bool `json:"candidate_diagnostics"`
	StatsRetention       int  `json:"stats_retention"`

	HotkeyEmoji       string `json:"hotkey_emoji"`
	HotkeyPunct       string `json:"hotkey_punct"`
	HotkeyTraditional string `json:"hotkey_traditional"`
	HotkeyMode        string `json:"hotkey_mode"`

	TrayIcon                bool   `json:"tray_icon"`
	StatusBar               bool   `json:"status_bar"`
	StatusBarAutohide       bool   `json:"status_bar_autohide"`
	BarScale                int    `json:"bar_scale"`
	BarIconSize             int    `json:"bar_icon_size"`
	BarHideSeconds          int    `json:"bar_hide_seconds"`
	BarShadow               bool   `json:"bar_shadow"`
	BarParts                int    `json:"bar_parts"`
	HintOnToggle            bool   `json:"hint_on_toggle"`
	HintOnFocus             bool   `json:"hint_on_focus"`
	HintPosition            string `json:"hint_position"`
	AutoUpdate              bool   `json:"auto_update"`
	ShowSettingsAfterUpdate bool   `json:"show_settings_after_update"`
}

// defaultConfig 和服务进程的默认值一样（连不上服务进程时界面先按它画）。
func defaultConfig() Config {
	return Config{
		EnglishCorrection: true, EnglishSpace: true, EnglishGloss: true, HotkeyEnglish: "off", HotkeyRadical: "off",
		PageSize: 7, Horizontal: true, FontSize: 18, FontFamily: "Microsoft YaHei", CandidateStyle: "pink", CandidateSkin: defaultCandidateSkin(), BrandIcon: "default", PinBadge: "paperclip",
		InlinePreedit: "segmented", CandidatePreedit: true, CandidateFollow: true, CandidateSingleLine: true, SentenceEditorPosition: "below",
		FuzzyGroups: fuzzyDefault, ShuangpinScheme: "xiaohe", TypoCorrection: true, TypoKinds: 15,
		ShiftToggle: "both", PageKeys: pageKeyMinus, SelectKeys: "off", PickChar: true, ArrowKeys: "candidates", NumpadDot: true,
		FullwidthPunct: true, DigitHalfwidthPunct: true, DigitPunctExceptions: true, DigitPunctFullwidth: digitPunctDefault,
		AutoPair: true, Autospace: true, AutospaceLetters: true, AutospaceDigits: true,
		DefaultMode: "chinese", ModeScope: "global",
		AppPresetRules: []AppPreset{}, VimModeApps: []string{},
		QuickPhrases: true, EmojiSuggest: true, DateTime: true, Calculator: true, Toolbar: true, EmailCompletion: true, LongWordCompletion: true,
		PredictSpace: true, PredictDoubleSpace: true, PredictArrows: true, StatsRetention: 365,
		HotkeyEmoji: "off", HotkeyPunct: "off", HotkeyTraditional: "off", HotkeyMode: "off",
		TrayIcon: true, HintOnToggle: true, HintPosition: "top-left", BarScale: 100, BarIconSize: 15, BarHideSeconds: 4, BarShadow: true, BarParts: 11,
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
	{4096, "an = ai"}, {8192, "huang = wang"}, {16384, "un = iong"}, {32768, "eng = ong"},
}

// 数字后仍保持全角的标点位掩码（和服务进程一致，默认勾选顿号）。
const digitPunctDefault = 1 << 0

var digitPunctRules = []struct {
	Bit   int
	Label string
}{
	{1 << 0, "、 顿号"},
	{1 << 1, "： 冒号"},
	{1 << 2, "， 逗号"},
	{1 << 3, "。 句号"},
	{1 << 4, "； 分号"},
	{1 << 5, "？ 问号"},
	{1 << 6, "！ 感叹号"},
	{1 << 7, "“ ” 双引号"},
	{1 << 8, "‘ ’ 单引号"},
	{1 << 9, "（ ） 圆括号"},
	{1 << 10, "【 】 方括号"},
	{1 << 11, "《 》 书名号"},
	{1 << 12, "「 」 直角引号"},
	{1 << 13, "—— 破折号"},
	{1 << 14, "…… 省略号"},
	{1 << 15, "～ 波浪号"},
	{1 << 16, "· 间隔号"},
	{1 << 17, "￥ 人民币"},
}

// AppPreset 是一次切入指定程序时要使用的中英文状态。
type AppPreset struct {
	App  string `json:"app"`
	Mode string `json:"mode"`
}

// AppWindow 是服务端发现的一个当前打开窗口所属程序。
type AppWindow struct {
	App   string `json:"app"`
	Title string `json:"title"`
}

// QuickPhrase 是一条快捷短语。
type QuickPhrase struct {
	Code   string `json:"code"`
	Text   string `json:"text"`
	Weight int    `json:"weight"`
}

// Dictionary 是一份导入的词库。
type Dictionary struct {
	Name        string `json:"name"`
	Entries     int    `json:"entries"`
	Enabled     bool   `json:"enabled"`
	Source      string `json:"source"`
	Description string `json:"description"`
}

type DictionaryEntry struct {
	Word   string `json:"word"`
	Pinyin string `json:"pinyin"`
	Weight uint32 `json:"weight"`
}

type DictionaryPage struct {
	Dictionary Dictionary        `json:"dictionary"`
	Total      int               `json:"total"`
	Offset     int               `json:"offset"`
	Entries    []DictionaryEntry `json:"entries"`
}

type SyncMergePreview struct {
	Versions  int      `json:"versions"`
	Added     int      `json:"added"`
	Updated   int      `json:"updated"`
	Deleted   int      `json:"deleted"`
	Conflicts int      `json:"conflicts"`
	Details   []string `json:"details"`
}

// UserWord 是一个学到的词。
type UserWord struct {
	Word    string  `json:"word"`
	Display string  `json:"display,omitempty"`
	Pinyin  string  `json:"pinyin"`
	Freq    float64 `json:"freq"`
}

func (word UserWord) text() string {
	if word.Display != "" {
		return word.Display
	}
	return word.Word
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
	NeuralModel  string `json:"neural_model"`
	Error        string `json:"error,omitempty"`

	LexiconWarning string `json:"lexicon_warning,omitempty"`
	LexiconLoading bool   `json:"lexicon_loading,omitempty"`
	// InputIdleSeconds 是距上次实际输入的秒数；没打过字或受限客户端为 nil。
	InputIdleSeconds *int64 `json:"input_idle_seconds,omitempty"`
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

type typingDay struct {
	Characters [5]uint64      `json:"characters"`
	ActiveMS   uint64         `json:"active_ms"`
	Hours      [24]uint64     `json:"hours"`
	Keys       map[int]uint64 `json:"keys"`
	Candidates [11]uint64     `json:"candidates"`
}

// Response 是一条应答。
type Response struct {
	Enhancement          *EnhancementStatus      `json:"enhancement,omitempty"`
	DictionaryUpdate     *DictionaryUpdateStatus `json:"dictionary_update,omitempty"`
	CandidateDiagnostics *CandidateDiagnostics   `json:"candidate_diagnostics,omitempty"`
	OK                   bool                    `json:"ok"`
	ModifiedUnixMS       int64                   `json:"modified_unix_ms,omitempty"`
	Error                string                  `json:"error,omitempty"`
	Status               *Status                 `json:"status,omitempty"`
	Config               *Config                 `json:"config,omitempty"`
	QuickPhrases         []QuickPhrase           `json:"quick_phrases,omitempty"`
	CommonPhrases        []string                `json:"common_phrases,omitempty"`
	Dictionaries         []Dictionary            `json:"dictionaries,omitempty"`
	DictionaryPage       *DictionaryPage         `json:"dictionary_page,omitempty"`
	SyncMerge            *SyncMergePreview       `json:"sync_merge,omitempty"`
	SyncFormat           int                     `json:"sync_format,omitempty"`
	UserWords            []UserWord              `json:"user_words,omitempty"`
	Report               *ImportReport           `json:"report,omitempty"`
	Frame                *Frame                  `json:"frame,omitempty"`
	Apps                 []AppWindow             `json:"apps,omitempty"`
	TypingStats          map[string]typingDay    `json:"typing_stats,omitempty"`
	ShuangpinExample     string                  `json:"shuangpin_example,omitempty"`
}

type CandidateDiagnostics struct {
	Bytes   uint64 `json:"bytes"`
	Dropped uint64 `json:"dropped"`
	Error   string `json:"error"`
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
	candidateStyleChoices = []choice{{"pink", "默认粉色"}, {"blue", "蓝色"}, {"green", "微信绿"}, {"sogou", "搜狗样式 无底色块"}, {"custom", "自定义配色"}}
	shiftToggles          = []choice{{"both", "左 Shift 或右 Shift"}, {"left", "仅左 Shift"}, {"right", "仅右 Shift"}, {"off", "关闭 Shift 切换"}}
	selectKeys            = []choice{{"off", "关闭"}, {"semicolon", "分号选第 2 项"}, {"quote", "分号、引号选第 2、3 项"}}
	arrowKeys             = []choice{{"candidates", "移动候选高亮"}, {"caret", "移动拼音光标"}}
	modes                 = []choice{{"chinese", "中文"}, {"english", "英文"}, {"global", "全局"}}
	modeScopes            = []choice{{"global", "所有应用一致"}, {"app", "每个应用分别记住"}}
	positions             = []choice{
		{"top-left", "左上方"}, {"top", "正上方"}, {"top-right", "右上方"},
		{"bottom-left", "左下方"}, {"bottom", "正下方"}, {"bottom-right", "右下方"},
	}
	hotkeysEmoji       = []choice{{"off", "关闭"}, {"ctrl+shift+e", "Ctrl + Shift + E"}, {"ctrl+alt+e", "Ctrl + Alt + E"}}
	hotkeysPunct       = []choice{{"off", "关闭"}, {"ctrl+period", "Ctrl + ."}}
	hotkeysTraditional = []choice{{"off", "关闭"}, {"ctrl+shift+f", "Ctrl + Shift + F"}, {"ctrl+shift+t", "Ctrl + Shift + T"}}
	hotkeysMode        = []choice{{"off", "关闭"}, {"ctrl+space", "Ctrl + 空格"}}
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
