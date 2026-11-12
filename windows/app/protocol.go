package main

// 服务进程管道消息（与 windows/protocol/src/lib.rs 一致）：每条请求、应答各一行 JSON。

// Config 是用户设置，字段名与服务进程的 config.json 相同。
type Config struct {
	PageSize        int    `json:"page_size"`
	Horizontal      bool   `json:"horizontal"`
	Fuzzy           bool   `json:"fuzzy"`
	Shuangpin       bool   `json:"shuangpin"`
	ShuangpinScheme string `json:"shuangpin_scheme"`
	Traditional     bool   `json:"traditional"`
	FullwidthPunct  bool   `json:"fullwidth_punct"`
	EmojiSuggest    bool   `json:"emoji_suggest"`
	DateTime        bool   `json:"date_time"`
	Calculator      bool   `json:"calculator"`
	TypoCorrection  bool   `json:"typo_correction"`
	QuickPhrases    bool   `json:"quick_phrases"`
	FontSize        int    `json:"font_size"`
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

// Status 是服务进程的状态。
type Status struct {
	Protocol int    `json:"protocol"`
	Version  string `json:"version"`
	Ready    bool   `json:"ready"`
	Mode     string `json:"mode"`
	DataDir  string `json:"data_dir"`
	Error    string `json:"error,omitempty"`
}

// ImportReport 是导入、导出的结果。
type ImportReport struct {
	Added   int      `json:"added"`
	Skipped int      `json:"skipped"`
	Errors  []string `json:"errors"`
}

// Frame 只取设置程序用到的字段。
type Frame struct {
	Mode string `json:"mode"`
}

// Response 是一条应答。
type Response struct {
	OK           bool          `json:"ok"`
	Error        string        `json:"error,omitempty"`
	Status       *Status       `json:"status,omitempty"`
	Config       *Config       `json:"config,omitempty"`
	QuickPhrases []QuickPhrase `json:"quick_phrases,omitempty"`
	Dictionaries []Dictionary  `json:"dictionaries,omitempty"`
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

// Backend 发请求、收应答；测试里换成假的。
type Backend interface {
	Call(Request) (Response, error)
}

// Shuangpin schemes: id and the name users see.
var schemes = []struct{ ID, Name string }{
	{"xiaohe", "小鹤双拼"},
	{"ziranma", "自然码"},
	{"microsoft", "微软双拼"},
	{"sogou", "搜狗双拼"},
	{"abc", "智能 ABC"},
	{"ziguang", "紫光双拼"},
	{"pinyinjiajia", "拼音加加"},
}
