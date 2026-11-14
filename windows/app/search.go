package main

import (
	"github.com/egoist/mygo/ui"
	"strings"
)

// 名称、说明和搜索索引共用一份数据，避免改文案后搜索结果指向旧名称。
type settingItem struct{ ID, Title, Description, Section, Keywords string }

var settingsItems = []settingItem{
	{"general/中文 / 英文", "中文 / 英文", "选择当前输入状态；切换键可在下方设置", "现在的状态", ""},
	{"general/标点", "标点", "中文状态下输入的标点", "现在的状态", ""},
	{"general/简体 / 繁体", "简体 / 繁体", "输出的字形", "现在的状态", ""},
	{"general/切换键", "切换键", "选择用哪一侧的 Shift 切换中英文，或关闭此按键", "中英文切换", "Shift 左 Shift 右 Shift"},
	{"general/默认状态", "默认状态", "选「全局」时，所有应用沿用最后主动切换的中英文状态，重启后也保留", "中英文切换", "中文 英文 全局"},
	{"general/中英文状态", "中英文状态", "选「每个应用分别记住」时，每个程序保留各自上次的中英文状态", "中英文切换", "全局统一 每个应用分别记住"},
	{"general/显示托盘图标", "显示托盘图标", "在任务栏通知区域显示 FreeType 图标。右键可切换状态或打开设置；也可从开始菜单打开设置", "状态提示", ""},
	{"general/悬浮状态栏", "悬浮状态栏", "在桌面上显示中 / 英、标点、简繁按钮，可拖动；右键可以隐藏", "状态提示", ""},
	{"general/自动隐藏", "自动隐藏", "空闲时隐藏悬浮状态栏，状态改变时重新显示", "状态提示", ""},
	{"general/切换状态时提示", "切换状态时提示", "切换中英文、标点、简繁时，在光标旁短暂显示新的状态", "状态提示", ""},
	{"general/切换输入框时提示", "切换输入框时提示", "光标移到另一个输入框（同一窗口或别的窗口）时，提示当前的中英文状态", "状态提示", ""},
	{"general/提示位置", "提示位置", "提示显示在光标的哪一边", "状态提示", "左上 正上 右上 左下 正下 右下 光标"},
	{"candidates/字体", "字体", "候选窗、提示使用的字体，默认是 Microsoft YaHei（微软雅黑）", "外观", "微软雅黑 Microsoft YaHei SimSun 宋体 KaiTi 楷体"},
	{"candidates/字号", "字号", "设置 100% 显示缩放下的字号，范围为 12–36 像素", "外观", ""},
	{"candidates/候选强调色", "候选强调色", "选择当前候选的标记样式。搜狗样式使用蓝色文字", "外观", "粉色 蓝色 绿色 搜狗 微信"},
	{"candidates/每页候选个数", "每页候选个数", "", "外观", ""},
	{"candidates/排列方向", "排列方向", "横排时 ← → 选词、↓ 展开更多候选；竖排时 ↑ ↓ 选词", "外观", ""},
	{"typing/双拼", "双拼", "使用双拼按键输入；编辑区显示展开后的拼音", "拼音", ""},
	{"typing/双拼方案", "双拼方案", "", "拼音", "小鹤 自然码 微软 搜狗 智能ABC 紫光 拼音加加"},
	{"typing/双拼展示键位", "双拼展示键位", "编辑区直接显示按下的双拼键位，不展开成全拼", "拼音", "双拼 按键 键位 提示"},
	{"typing/模糊音", "模糊音", "按所选规则匹配相近读音，例如启用 z / zh 后，zi 可匹配「知」", "拼音", ""},
	{"typing/按错键纠错", "按错键纠错", "为相邻按键和交换按键等输入错误提供候选，例如 weixni → 微信", "拼音", ""},
	{"typing/显示更多生僻字", "显示更多生僻字", "显示 CJK 扩展 B 区及后续区的单字。字体不支持时，这些字可能显示为方块", "拼音", ""},
	{"typing/翻页键", "翻页键", "选择额外的翻页键。PgUp、PgDn 固定用于翻页", "选词与翻页", "PgUp PgDn Page Up Page Down Tab - = [ ] , ."},
	{"typing/分号、引号选词", "分号、引号选词", "", "选词与翻页", "第二 第三 2 3 ; '"},
	{"typing/左右方向键", "左右方向键", "移动候选高亮，或在拼音里移动光标修改拼写；后者用 ↑ ↓ 选词，Home、End、Delete 也作用于拼音", "选词与翻页", "光标 左移 右移 Home End Delete 方向键 候选"},
	{"typing/以词定字", "以词定字", "打拼音时按 [ 上屏候选的第一个字，按 ] 上屏最后一个字（翻页键用了 [ ] 时让给翻页）", "选词与翻页", ""},
	{"typing/中文标点", "中文标点", "中文状态下输入 ，。？ 这类全角标点；数字后面的标点保持半角（3.14）", "标点", ""},
	{"typing/成对标点自动补全", "成对标点自动补全", "输入 （ “ 《 【 「 时自动补上另一半，光标放在中间", "标点", ""},
	{"typing/小键盘的点输入「.」", "小键盘的点输入「.」", "小键盘的点总是半角；打字途中按它进入网址、邮箱输入", "标点", ""},
	{"typing/中英文之间自动加空格", "中英文之间自动加空格", "例如 打tag → 打 tag。处理同一段混输和光标前的文字边界；应用无法提供光标前文字时，处理本次上屏内容", "中英文混排", ""},
	{"typing/汉字和字母之间", "汉字和字母之间", "", "中英文混排", ""},
	{"typing/汉字和数字之间", "汉字和数字之间", "", "中英文混排", ""},
	{"features/v + 空格打开工具栏", "v + 空格打开工具栏", "全拼下只打 v 再按空格：剪贴板、常用语、符号、表情", "工具栏与表情", ""},
	{"features/记录剪贴板历史", "记录剪贴板历史", "在内存中保存最近 20 条，关闭服务后清除。应用标记为禁止记录的内容会跳过", "工具栏与表情", ""},
	{"features/清空剪贴板历史", "清空剪贴板历史", "", "工具栏与表情", ""},
	{"features/推荐 emoji", "推荐 emoji", "打「开心」时候选里出现 😊，打「八」时出现 8️⃣", "工具栏与表情", ""},
	{"features/邮箱、网址补全", "邮箱、网址补全", "输入 @ 显示 @qq.com、@gmail.com 等后缀；输入 gmail. 或 www.xxx. 显示 .com 等结尾", "智能输入", ""},
	{"features/日期时间", "日期时间", "输入 riqi、shijian、xingqi（或 rq、sj）得到现在的日期、时间、星期", "智能输入", ""},
	{"features/V 模式：计算与数字大写", "V 模式：计算与数字大写", "v1+2*3 得到 7；v1314 得到 壹仟叁佰壹拾肆元整 和 一千三百一十四（双拼下用大写 V 开头）", "智能输入", "双拼 大写 V 计算器"},
	{"features/快捷短语混入候选", "快捷短语候选", "输入完整编码后，将对应的快捷短语放在候选前面", "智能输入", ""},
	{"features/输出繁体", "输出繁体", "候选和上屏的字都换成繁体", "智能输入", ""},
	{"shortcuts/表情面板", "表情面板", "打开表情面板，可用拼音或英文搜索表情", "可以设置的快捷键", ""},
	{"shortcuts/中文 / 英文标点", "中文 / 英文标点", "", "可以设置的快捷键", ""},
	{"shortcuts/简体 / 繁体", "简体 / 繁体", "", "可以设置的快捷键", ""},
	{"shortcuts/切换中文 / 英文", "切换中文 / 英文", "设置 Shift 以外的中英文切换快捷键", "可以设置的快捷键", ""},
	{"dicts/导入词库", "导入词库", "自动识别搜狗细胞词库（.scel）、搜狗文本词库、水杉、Rime 和三列 TSV 格式。只有词语的文件会按常用读音自动注音", "导入词库", ""},
	{"phrases/导入或导出", "导入或导出", "导入自动识别：水杉（编码<Tab>短语）、Rime（短语<Tab>编码）、搜狗自定义短语和微信输入法常用语导出（编码,位置=短语）。导出为水杉格式", "批量导入、导出", ""},
	{"learned/学习备份", "学习备份", "保存词条、选词次数和搭配；导入时保留现有记录，重复导入不累加", "", ""},
	{"learned/全部清空", "全部清空", "删除全部学习词条、选词次数和搭配，恢复词库原有排序", "", ""},
	{"about/日志", "日志", "打开诊断文件夹，其中包含服务日志 server.log 和输入法日志 tsf.log", "数据与日志", ""},
	{"about/GitHub", "GitHub", repositoryURL, "关于", "仓库 源代码 项目"},
	{"about/问题反馈", "问题反馈", repositoryURL + "/issues", "关于", "GitHub 问题 bug 反馈"},
	{"about/检查更新", "检查更新", "检查已发布的 Windows 安装器，并可立即下载和更新", "软件更新", "版本 下载 安装 手动 立即更新"},
	{"about/自动更新", "自动更新", "默认关闭。开启后，每次启动检查更新，运行期间每 24 小时检查一次；有新版本时后台下载，下次启动 FreeType 时安装", "软件更新", "后台 下载 自动 重启 升级"},
	{"about/更新安装后显示设置页", "更新安装后显示设置页", "默认关闭。开启后，安装或更新完成时自动打开设置页", "软件更新", "窗口 启动 弹出 升级 安装完成"},
	{"about/开源信息", "开源信息", "查看项目的第三方来源、许可和使用范围", "来源与许可", "来源 授权"},
	{"about/词库", "词库许可", "词库和语言模型的来源与许可保存在安装目录的 data 文件夹中", "来源与许可", "LICENSE SOURCE.txt 白霜 万象"},
	{"common/导入或导出", "导入或导出", "导出为 JSON，保留多行内容；也能导入 UTF-8 文本，一行一条。重复内容只保留一条", "批量导入、导出", ""},
	{"general/全部设置和数据", "全部设置", "保存或恢复设置、词库、快捷短语、常用语和学习记录", "一键导入、导出", "数据 输入习惯"},
	{"typing/模糊音分组", "模糊音分组", "选择要合并的相近读音", "拼音", "z zh c ch s sh n l f h r l an ang en eng in ing ian iang uan uang un ong ai huang wang iong"},
	{"dicts/词库文件", "词库文件", "选择自定义主词库，或恢复白霜、万象和项目热词组成的默认词库", "主词库", "白霜 万象 热词 拼音 lexicon.tsv 选择文件 恢复默认"},
	{"dicts/已导入词库", "已导入词库", "查看或删除导入的词库", "词库", "删除 管理"},
	{"phrases/添加快捷短语", "添加快捷短语", "设置编码、短语和权重；支持多行内容", "快捷短语", "编码 内容 权重 添加"},
	{"phrases/全部快捷短语", "全部快捷短语", "搜索或删除快捷短语", "快捷短语", "管理 搜索 编码"},
	{"common/添加常用语", "添加常用语", "内容支持多行，不需要编码", "常用语", "工具栏 v 空格 添加"},
	{"common/全部常用语", "全部常用语", "搜索或删除常用语", "常用语", "管理 搜索"},
	{"learned/添加词", "添加词", "添加中文、英文或中英混输词，填写拼音或实际输入码", "学到的词", "词语 拼音 输入码 英文 中英混输 添加"},
	{"learned/词条列表", "词条列表", "搜索、刷新或删除学到的词", "学到的词", "个人 习惯 词频"},
	{"shortcuts/固定按键", "固定按键", "查看选词、翻页、删除记忆和工具栏按键", "快捷键", "Shift 空格 1 9 Enter 回车 PgUp PgDn Ctrl Delete Esc v"},
	{"about/用户数据", "用户数据", "打开保存个人设置和词库的文件夹", "关于", "备份 文件夹"},
}

var settingsByID = func() map[string]settingItem {
	items := make(map[string]settingItem, len(settingsItems))
	for _, item := range settingsItems {
		items[item.ID] = item
	}
	return items
}()

func setting(id string) settingItem {
	item, ok := settingsByID[id]
	if !ok {
		panic("unknown setting: " + id)
	}
	return item
}

func pageName(id string) string {
	for _, page := range pages {
		if page.ID == id {
			return page.Name
		}
	}
	return id
}

func findSettings(query string) []settingItem {
	terms := strings.Fields(strings.ToLower(query))
	var found []settingItem
	if len(terms) == 0 {
		return found
	}
	for _, item := range settingsItems {
		page, _, _ := strings.Cut(item.ID, "/")
		text := strings.ToLower(pageName(page) + " " + item.Title + " " + item.Description + " " + item.Section + " " + item.Keywords)
		match := true
		for _, term := range terms {
			if !strings.Contains(text, term) {
				match = false
				break
			}
		}
		if match {
			found = append(found, item)
		}
	}
	return found
}

func (s *settings) searchField(c *ui.Context) {
	ui.Column(c).FillWidth().Children(func() {
		ui.SearchField(c, &s.settingQuery).Label("搜索全部设置").Placeholder("搜索设置").FillWidth().OnTextKey(func(key ui.TextKey) bool {
			if key.Mods != 0 {
				return false
			}
			switch key.Key {
			case ui.KeyEscape:
				key.SetText("")
				s.settingQuery = ""
				return true
			case ui.KeyEnter:
				if found := findSettings(key.Text()); len(found) > 0 {
					s.locateSetting(found[0])
					key.SetText("")
				}
				return true
			}
			return false
		})
	})
}

func (s *settings) searchPage(c *ui.Context) {
	pageHeader(c, "搜索设置", "选择结果后跳转并定位对应设置")
	found := findSettings(s.settingQuery)
	if len(found) == 0 {
		ui.Text(c, "没有找到相关设置").Margin(20, 0)
		return
	}
	ui.Textf(c, "找到 %d 项", len(found)).TextColor(colors(c).muted).Margin(8, 0)
	for _, item := range found {
		page, _, _ := strings.Cut(item.ID, "/")
		result := ui.Column(c).Key(item.ID).Padding(14, 18).Gap(5).Margin(4, 0).Radius(6).
			Background(colors(c).card).Border(1, colors(c).border).Cursor(ui.CursorPointer).
			Label("定位 " + pageName(page) + " / " + item.Title)
		result.Children(func() {
			ui.Text(c, item.Title).FontSize(15).FontWeight(500)
			ui.Text(c, pageName(page)+" · "+item.Section).FontSize(13).TextColor(colors(c).muted)
			if item.Description != "" {
				ui.Text(c, item.Description).FontSize(13).TextColor(colors(c).muted).Wrap()
			}
		})
		if result.Clicked() {
			s.locateSetting(item)
		}
	}
}

func (s *settings) locateSetting(item settingItem) {
	page, _, _ := strings.Cut(item.ID, "/")
	s.page, s.settingQuery, s.locatedSetting, s.revealSetting = page, "", item.ID, true
	if page == "learned" && !s.wordsLoaded {
		s.loadWords()
	}
}

func (s *settings) showSetting(id string) bool { return s.locatedSetting == id }

// 定位隐藏选项只改变显示，不替用户打开它的前提开关。
func (s *settings) unavailableReason(id string) string {
	switch id {
	case "typing/双拼方案":
		if !s.config.Shuangpin {
			return "请先开启双拼，再选择方案。"
		}
	case "typing/双拼展示键位":
		if !s.config.Shuangpin {
			return "请先开启双拼。"
		}
	case "typing/模糊音分组":
		if !s.config.Fuzzy {
			return "请先开启模糊音，再选择分组。"
		}
	case "typing/汉字和字母之间", "typing/汉字和数字之间":
		if !s.config.Autospace {
			return "请先开启中英文之间自动加空格。"
		}
	case "general/自动隐藏":
		if !s.config.StatusBar {
			return "请先开启悬浮状态栏。"
		}
	case "general/中英文状态":
		if s.config.DefaultMode == "global" {
			return "默认状态为全局时，所有应用共用中英文状态。"
		}
	case "features/清空剪贴板历史":
		if !s.config.ClipboardHistory {
			return "请先开启记录剪贴板历史。"
		}
	}
	return ""
}

func (s *settings) settingBlock(c *ui.Context, id string, content func()) {
	item := setting(id)
	block := ui.Column(c).Key(id).FillWidth().Label("设置项：" + item.Title)
	chosen := s.locatedSetting == id
	if chosen {
		block.Bind(&s.locatedHandle)
	}
	reason := s.unavailableReason(id)
	block.Children(func() {
		if reason == "" {
			content()
		} else {
			ui.Column(c).FillWidth().Disabled(true).Children(content)
		}
		if chosen && reason != "" {
			ui.Text(c, reason).FontSize(13).TextColor(colors(c).muted).Wrap().Padding(0, 18, 12, 18)
		}
	})
	if chosen && s.revealSetting {
		block.ScrollIntoView(ui.ScrollIntoViewOptions{Block: ui.ScrollCenter})
		s.revealSetting = false
	}
}

func (s *settings) settingRow(c *ui.Context, id string, control func()) {
	item := setting(id)
	s.settingBlock(c, id, func() { row(c, item.Title, item.Description, control) })
}

func (s *settings) settingRowValue(c *ui.Context, id, value string, control func()) {
	s.settingBlock(c, id, func() { row(c, setting(id).Title, value, control) })
}

func (s *settings) settingSubRow(c *ui.Context, id string, control func()) {
	item := setting(id)
	s.settingBlock(c, id, func() { subRow(c, item.Title, item.Description, control) })
}

func (s *settings) settingSwitch(c *ui.Context, id string, sub bool, value *bool) bool {
	item := setting(id)
	changed := false
	s.settingBlock(c, id, func() { changed = switchLine(c, item.Title, item.Description, value, sub) })
	return changed
}
