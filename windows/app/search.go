package main

import (
	"github.com/egoist/mygo/ui"
	"strings"
)

// 名称、说明和搜索索引共用一份数据，避免改文案后搜索结果指向旧名称。
type settingItem struct{ ID, Title, Description, Section, Keywords string }

var settingsItems = []settingItem{
	{"general/默认输入法", "默认输入法", "", "常规", "Windows 系统设置 语言"},
	{"general/中文 / 英文", "中文 / 英文", "选择当前输入状态；切换键可在下方设置", "现在的状态", ""},
	{"general/标点", "标点", "中文状态下输入的标点", "现在的状态", ""},
	{"general/简体 / 繁体", "简体 / 繁体", "输出的字形", "现在的状态", ""},
	{"general/切换键", "切换键", "选择用哪一侧的 Shift 切换中英文，或关闭此按键", "中英文切换", "Shift 左 Shift 右 Shift"},
	{"general/默认状态", "默认状态", "选「全局」时，所有应用沿用最后主动切换的中英文状态，重启后也保留", "中英文切换", "中文 英文 全局"},
	{"general/中英文状态", "中英文状态", "选「每个应用分别记住」时，每个程序保留各自上次的中英文状态", "中英文切换", "全局统一 每个应用分别记住"},
	{"features/指定软件自动切换", "指定软件自动切换", "切入指定程序时切换一次，内部可手动切换；离开后恢复原模式，规则会保存", "指定软件自动切换", "预设 英文 中文 程序 应用 exe"},
	{"features/指定软件规则", "指定软件规则", "从当前打开的窗口选择程序，或输入 exe 文件名，设置切入时使用中文还是英文", "指定软件自动切换", "打开窗口 进程 添加 更新"},
	{"features/指定软件规则列表", "指定软件规则列表", "查看、修改或删除已记住的指定程序规则", "指定软件自动切换", "删除 修改 规则 记住"},
	{"general/显示托盘图标", "显示托盘图标", "在任务栏通知区域显示 FreeType 图标。右键可切换状态或打开设置；也可从开始菜单打开设置", "状态提示", ""},
	{"general/悬浮状态栏", "悬浮状态栏", "在桌面上显示中 / 英、标点、简繁按钮，可拖动；右键可以隐藏", "状态提示", ""},
	{"general/自动隐藏", "自动隐藏", "空闲时隐藏悬浮状态栏，状态改变时重新显示", "状态提示", ""},
	{"general/切换状态时提示", "切换状态时提示", "切换中英文、标点、简繁时，在光标旁短暂显示新的状态", "状态提示", ""},
	{"general/切换输入框时提示", "切换输入框时提示", "光标移到另一个输入框（同一窗口或别的窗口）时，提示当前的中英文状态", "状态提示", ""},
	{"general/提示位置", "提示位置", "提示显示在光标的哪一边", "状态提示", "左上 正上 右上 左下 正下 右下 光标"},
	{"candidates/品牌图标", "品牌图标", "立即更新设置窗口和托盘；更新系统输入指示图标时需要管理员授权。自定义支持 PNG、JPEG、WebP，导入后不再依赖原文件", "品牌外观", "icon logo 品牌 白色 默认 自定义 图标 外观 语言栏"},
	{"candidates/字体", "字体", "候选窗、提示使用的字体，默认是 Microsoft YaHei（微软雅黑）", "外观", "微软雅黑 Microsoft YaHei SimSun 宋体 KaiTi 楷体"},
	{"candidates/字号", "字号", "设置 100% 显示缩放下的字号，范围为 12–36 像素", "外观", ""},
	{"candidates/候选强调色", "候选强调色", "选择当前候选的标记样式。搜狗样式使用蓝色文字", "外观", "粉色 蓝色 绿色 搜狗 微信"},
	{"candidates/每页候选个数", "每页候选个数", "", "外观", ""},
	{"candidates/滚轮翻页", "滚轮翻页", "需启用 Windows 鼠标设置中的非活动窗口滚动", "外观", "鼠标 翻页"},
	{"candidates/固定排位角标", "固定排位角标", "", "外观", "置顶 图钉 回形针 圆点"},
	{"candidates/排列方向", "排列方向", "横排时 ← → 选词、↓ 展开更多候选；竖排时 ↑ ↓ 选词", "外观", ""},
	{"candidates/行内预编辑", "行内预编辑", "", "预编辑与位置", "分词 原始按键"},
	{"candidates/候选窗预编辑", "候选窗预编辑", "", "预编辑与位置", "隐藏 拼音"},
	{"candidates/预编辑字号", "预编辑字号", "", "预编辑与位置", "大小"},
	{"candidates/跟随光标", "跟随光标", "", "预编辑与位置", "固定 位置"},
	{"candidates/横排保持单行", "横排保持单行", "", "预编辑与位置", "换行"},
	{"candidates/补充字体", "补充字体", "", "预编辑与位置", "回落 生僻字"},
	{"typing/双拼", "双拼", "使用双拼按键输入；编辑区显示展开后的拼音", "拼音", ""},
	{"typing/双拼方案", "双拼方案", "", "拼音", "小鹤 自然码 微软 搜狗 智能ABC 紫光 拼音加加"},
	{"typing/双拼展示键位", "双拼展示键位", "编辑区直接显示按下的双拼键位，不展开成全拼", "拼音", "双拼 按键 键位 提示"},
	{"typing/模糊音", "模糊音", "按所选规则匹配相近读音，例如启用 z / zh 后，zi 可匹配「知」", "拼音", ""},
	{"typing/按错键纠错", "按错键纠错", "为相邻按键和交换按键等输入错误提供候选，例如 weixni → 微信", "拼音", ""},
	{"typing/纠错类型", "纠错类型", "全拼下分别选择要纠正的按键错误", "拼音", "乱序 邻键 漏字 多字"},
	{"typing/纠错标记", "纠错标记", "在纠错得到的候选旁显示 *，不影响上屏文字", "拼音", "星号 *"},
	{"typing/显示更多生僻字", "显示更多生僻字", "显示 CJK 扩展 B 区及后续区的单字。字体不支持时，这些字可能显示为方块", "拼音", ""},
	{"typing/翻页键", "翻页键", "选择额外的翻页键。PgUp、PgDn 固定用于翻页", "选词与翻页", "PgUp PgDn Page Up Page Down Tab - = [ ] , ."},
	{"typing/分号、引号选词", "分号、引号选词", "", "选词与翻页", "第二 第三 2 3 ; '"},
	{"typing/左右方向键", "左右方向键", "移动候选高亮，或在拼音里移动光标修改拼写；后者用 ↑ ↓ 选词，Home、End、Delete 也作用于拼音", "选词与翻页", "光标 左移 右移 Home End Delete 方向键 候选"},
	{"typing/以词定字", "以词定字", "打拼音时按 [ 上屏候选的第一个字，按 ] 上屏最后一个字（翻页键用了 [ ] 时让给翻页）", "选词与翻页", ""},
	{"typing/中文标点", "中文标点", "中文状态下输入 ，。？ 这类全角标点", "标点", ""},
	{"typing/数字标点保持半角", "数字标点保持半角", "数字后面输入标点时保持半角（3.14、3,000、20:22）", "标点", "半角 全角 小数点 时间 比例"},
	{"typing/数字后例外保持全角", "数字后例外保持全角", "数字后面遇到下方勾选的标点时，仍输出中文全角符号（如 1、）", "标点", "顿号 冒号 例外 全角"},
	{"typing/数字后保持全角的标点", "数字后保持全角的标点", "勾选数字后面仍要保持全角的常用符号", "标点", "、 ： ， 。 ； ？ ！ “ ” ‘ ’ （ ） 【 】 《 》 「 」 —— …… ～ · ￥ 顿号 冒号 逗号 句号 分号 问号 感叹号 双引号 单引号 圆括号 方括号 书名号 直角引号 破折号 省略号 波浪号 间隔号 人民币"},
	{"typing/双引号改用直角引号", "双引号改用直角引号", "中文标点下按 \" 输入直角引号「」，替代默认弯引号“ ”", "标点", "弯引号 直角引号 双引号 引号 「 」 “ ”"},
	{"typing/成对标点自动补全", "成对标点自动补全", "输入 （ “ 《 【 「 时自动补上另一半，光标放在中间", "标点", ""},
	{"typing/小键盘的点输入「.」", "小键盘的点输入「.」", "小键盘的点总是半角；打字途中按它进入网址、邮箱输入", "标点", ""},
	{"typing/中英文之间自动加空格", "中英文之间自动加空格", "例如 打tag → 打 tag。处理同一段混输和光标前的文字边界；应用无法提供光标前文字时，处理本次上屏内容", "中英文混排", ""},
	{"typing/汉字和字母之间", "汉字和字母之间", "", "中英文混排", ""},
	{"typing/汉字和数字之间", "汉字和数字之间", "", "中英文混排", ""},
	{"features/v + 空格打开工具栏", "v + 空格打开工具栏", "全拼下只打 v 再按空格：剪贴板、常用语、符号、表情", "工具栏与表情", ""},
	{"features/记录剪贴板历史", "记录剪贴板历史", "在内存中保存最近 20 条，关闭服务后清除。应用标记为禁止记录的内容会跳过", "工具栏与表情", ""},
	{"features/清空剪贴板历史", "清空剪贴板历史", "", "工具栏与表情", ""},
	{"features/推荐 emoji", "推荐 emoji", "打「开心」时候选里出现 😊，打「八」时出现 8️⃣", "工具栏与表情", ""},
	{"features/整句增强模型", "整句增强模型", "", "智能输入", "NNLM 微信 导入"},
	{"features/邮箱、网址补全", "邮箱、网址补全", "输入 @ 显示 @qq.com、@gmail.com 等后缀；输入 gmail. 或 www.xxx. 显示 .com 等结尾", "智能输入", ""},
	{"features/日期时间", "日期时间", "输入 riqi、shijian、xingqi（或 rq、sj）得到现在的日期、时间、星期", "智能输入", ""},
	{"features/V 模式：计算与数字大写", "V 模式：计算与数字大写", "v1+2*3 得到 7；v1314 得到 壹仟叁佰壹拾肆元整 和 一千三百一十四（双拼下用大写 V 开头）", "智能输入", "双拼 大写 V 计算器"},
	{"features/快捷短语混入候选", "快捷短语候选", "输入完整编码后，将对应的快捷短语放在候选前面", "智能输入", ""},
	{"features/输出繁体", "输出繁体", "候选和上屏的字都换成繁体", "智能输入", ""},
	{"features/上屏联想", "上屏联想", "默认关闭。一个词上屏后，候选窗接着显示常跟在它后面的词；选中后继续联想下一个词。直接打字、按 Esc 或几秒不选会自动收起", "联想", "预测 下一个词 连续上屏 联想词 搭配"},
	{"features/数字键选联想词", "数字键选联想词", "按 1–9、0 上屏对应的联想词。关闭后数字照常输入", "联想", "预测 选词 1 9"},
	{"features/空格选联想词", "空格选联想词", "用空格上屏当前高亮的联想词。关闭后空格照常输入", "联想", "预测 选词 空格"},
	{"features/双击空格键选词", "双击空格键选词", "", "联想", "两次空格 双按"},
	{"features/左右方向键选联想词", "左右方向键选联想词", "默认开启。按 ← / → 移动联想词高亮；关闭后左右方向键照常移动正文光标", "联想", "预测 选词 方向键 左右 光标"},
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
	{"sync/全部设置和数据", "全部设置", "保存或恢复输入设置、词库和学习记录；导出时可选择包含哪些数据并可选设置备份密码加密，包含同步账号和计划，不含密码或密钥，导入后需重新填写同步密码", "一键导入导出", "数据 输入习惯 备份 账号 密码 加密 选择 部分 导出内容"},
	{"sync/启用同步", "启用同步", "默认关闭。开启后可以把全部设置和数据上传到你自己的 WebDAV 或 S3 兼容存储，再在别的电脑上合并恢复", "云端同步", "WebDAV S3 坚果云 MinIO R2 OSS 网盘 云 备份 多台电脑"},
	{"sync/同步服务", "同步服务", "填写 WebDAV 或 S3 兼容存储的地址和凭据，并测试连接", "云端同步", "WebDAV S3 地址 账号 密码 存储桶 Access Key Secret Key 测试连接"},
	{"sync/备份加密", "备份加密", "默认关闭。开启后，上传到网盘的备份用备份密码加密；备份里的电脑名和上传时间不加密", "云端同步", "安全 密码 加密 云端备份 明文 隐私 口令 AES"},
	{"sync/备份密码", "备份密码", "只保存在这台电脑上。每台电脑要填同一个密码；忘记后无法打开已加密的备份", "云端同步", "密码 加密 口令 解密"},
	{"sync/上传与恢复", "上传与恢复", "上传当前数据的快照；查看各台电脑的快照并合并到本机", "云端同步", "立即上传 刷新设备 恢复 合并 快照"},
	{"sync/自动同步", "自动同步", "按数据修改时间同步，较新的完整备份替换较旧数据", "云端同步", "定时 间隔 每天 次数 重试 自动上传 自动下载"},
	{"sync/同步计划", "同步计划", "设置时间、次数、失败重试和备份保留数", "云端同步", "分钟 每日 上限 备份 历史"},
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
	{"about/导出诊断", "导出诊断", "", "数据与日志", "脱敏 诊断 错误"},
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
		ui.Text(c, "没有找到相关设置").Margin(16, 0, 0, 0)
		hint(c, "试试更短的关键词，或从侧边栏选择设置页面。")
		return
	}
	ui.Textf(c, "找到 %d 项", len(found)).TextColor(colors(c).muted).Margin(8, 0)
	for _, item := range found {
		page, _, _ := strings.Cut(item.ID, "/")
		// 使用原生按钮的 Tab、Enter、Space 和焦点环，而不是仅响应鼠标的容器。
		result := ui.ButtonBase(c).Key(item.ID).Padding(14, rowInset).Margin(4, 0).Radius(panelRadius).
			FillWidth().AlignItems(ui.Start).Background(colors(c).card).Border(1, colors(c).border).Cursor(ui.CursorPointer).
			Label("定位 " + pageName(page) + " / " + item.Title)
		if !c.Preferences().HighContrast {
			alpha := float32(0.03)
			if c.Theme().Dark {
				alpha = 0.18
			}
			result.Shadow(0, 1, 3, 0, ui.RGBA(0, 0, 0, alpha))
		}
		if result.Hovered() {
			result.Background(colors(c).cardHover)
		}
		result.Children(func() {
			ui.Column(c).FillWidth().Gap(4).Children(func() {
				ui.Row(c).Gap(8).Wrap().AlignItems(ui.Center).Children(func() {
					ui.Text(c, item.Title).FontWeight(600).Wrap()
					breadcrumb := pageName(page)
					if item.Section != "" {
						breadcrumb += " · " + item.Section
					}
					ui.Text(c, breadcrumb).FontSize(textSize(c, 12)).TextColor(colors(c).muted).
						Padding(2, 8).Radius(controlRadius).Background(colors(c).subtle).SingleLine()
				})
				if item.Description != "" {
					ui.Text(c, item.Description).FontSize(textSize(c, 13)).TextColor(colors(c).muted).LineHeight(1.45).Wrap()
				}
			})
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
	case "typing/数字标点保持半角":
		if !s.config.FullwidthPunct {
			return "请先开启中文标点。"
		}
	case "typing/数字后例外保持全角":
		if !s.config.FullwidthPunct {
			return "请先开启中文标点。"
		}
		if !s.config.DigitHalfwidthPunct {
			return "请先开启数字标点保持半角。"
		}
	case "typing/数字后保持全角的标点":
		if !s.config.FullwidthPunct {
			return "请先开启中文标点。"
		}
		if !s.config.DigitHalfwidthPunct {
			return "请先开启数字标点保持半角。"
		}
		if !s.config.DigitPunctExceptions {
			return "请先开启数字后例外保持全角。"
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
	case "features/指定软件规则", "features/指定软件规则列表":
		if !s.config.AppPresets {
			return "请先开启指定软件自动切换。"
		}
	case "features/清空剪贴板历史":
		if !s.config.ClipboardHistory {
			return "请先开启记录剪贴板历史。"
		}
	case "sync/上传与恢复", "sync/自动同步", "sync/同步计划", "sync/备份加密", "sync/备份密码":
		if !s.sync.Enabled {
			return "请先启用云端同步。"
		}
	case "features/双击空格键选词":
		if !s.config.PredictNext || !s.config.PredictSpace {
			return "请先开启空格选联想词。"
		}
	case "features/数字键选联想词", "features/空格选联想词", "features/左右方向键选联想词":
		if !s.config.PredictNext {
			return "请先开启上屏联想。"
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
			ui.Text(c, reason).FontSize(textSize(c, 13)).TextColor(colors(c).muted).LineHeight(1.4).Wrap().Padding(0, rowInset, 14, rowInset)
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
