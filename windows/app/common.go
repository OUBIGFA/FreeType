package main

import (
	"fmt"
	"strings"

	"github.com/egoist/mygo/ui"
)

// 常用语独立于快捷短语：没有编码，只从 v + 空格的工具栏中选择。
func (s *settings) commonPage(c *ui.Context) {
	pageHeader(c, "常用语", "常用语无需编码；启用工具栏后，在全拼下按 v + 空格打开工具栏，再选常用语")
	section(c, "添加", func() {
		s.settingBlock(c, "common/添加常用语", func() {
			ui.Column(c).Padding(space4, 0).Gap(space4).Children(func() {
				ui.Column(c).Children(func() {
					ui.TextArea(c, &s.newCommon).Label("常用语内容").Placeholder("输入常用语，支持多行").Height(80)
				})
				if s.commonError != "" {
					ui.Text(c, s.commonError).TextColor(c.Theme().Danger).Wrap()
				}
				if ui.PrimaryButton(c, "添加常用语").AlignSelf(ui.Start).Disabled(s.commonBusy).Clicked() {
					s.addCommonPhrase()
				}
			})
		})
	})
	section(c, "批量导入、导出", func() {
		s.settingRow(c, "common/导入或导出", func() {
			checkbox(c, &s.replaceCommon, "替换现有常用语")
			if ui.Button(c, "导入").Disabled(s.commonBusy).Clicked() {
				s.importCommonPhrases()
			}
			if ui.Button(c, "导出").Clicked() {
				s.export("export_common_phrases", "导出常用语", "FreeType 常用语.json")
			}
		})
	})
	s.reportView(c)
	listHeader(c, fmt.Sprintf("常用语 %d 条", len(s.common)), func() {
		ui.SearchField(c, &s.commonQuery).Label("搜索常用语").Placeholder("搜索常用语").Width(240).MaxWidthPercent(100).Shrink(1)
	})
	section(c, "", func() {
		s.settingBlock(c, "common/全部常用语", func() {
			var shown []int
			query := strings.ToLower(strings.TrimSpace(s.commonQuery))
			for i, text := range s.common {
				if query == "" || strings.Contains(strings.ToLower(text), query) {
					shown = append(shown, i)
				}
			}
			if len(shown) == 0 {
				title := "没有常用语"
				if query != "" {
					title = "没有匹配的常用语"
				}
				row(c, title, "在上方添加内容，或导入已有文件", nil)
				return
			}
			remove := -1
			ui.List(c, nil, len(shown), func(k int) {
				i := shown[k]
				item := ui.Row(c).Height(phraseRow).Gap(space3).AlignItems(ui.Center)
				item.Children(func() {
					ui.Text(c, preview(s.common[i])).Grow(1).Basis(0).MinWidth(0).Shrink(1).SingleLine().Tooltip(s.common[i])
					onIntent(c, item, func() {
						if ui.Button(c, "删除").Label(fmt.Sprintf("删除常用语 %d", i+1)).Disabled(s.commonBusy).Clicked() {
							remove = i
						}
					})
				})
			}).MaxHeight(phraseRow * maxPhraseRows)
			if remove >= 0 {
				phrases := append(append([]string(nil), s.common[:remove]...), s.common[remove+1:]...)
				s.saveCommonPhrases(phrases, func() { s.notice = "已删除常用语" })
			}
		})
	})
}

func (s *settings) saveCommonPhrases(phrases []string, done func()) {
	if s.commonBusy {
		return
	}
	s.commonBusy = true
	// 协议中的列表必须是 []；删掉最后一条时不能发送 JSON null。
	if phrases == nil {
		phrases = []string{}
	}
	s.background(func() func() {
		_, err := s.backend.Call(op("set_common_phrases", "phrases", phrases))
		if err != nil {
			return func() { s.commonBusy = false; s.notice = "保存失败：" + err.Error() }
		}
		return func() {
			s.commonBusy = false
			// 这些条目已在添加时规范换行；确认写入后接纳该列表，不再多发读取请求。
			s.common = phrases
			done()
		}
	})
}

func (s *settings) addCommonPhrase() {
	if s.commonBusy {
		return
	}
	input := s.newCommon
	text := strings.ReplaceAll(input, "\r\n", "\n")
	if strings.TrimSpace(text) == "" || strings.ContainsRune(text, '\x00') {
		s.commonError = "常用语不能为空，也不能包含空字符"
		return
	}
	for _, existing := range s.common {
		if existing == text {
			s.commonError = "这条常用语已经存在"
			return
		}
	}
	s.commonError = ""
	phrases := append(append([]string(nil), s.common...), text)
	s.saveCommonPhrases(phrases, func() {
		if s.newCommon == input {
			s.newCommon = ""
		}
		s.notice = "已添加常用语"
	})
}

func (s *settings) importCommonPhrases() {
	if s.commonBusy {
		return
	}
	s.commonBusy = true
	replace := s.replaceCommon
	s.background(func() func() {
		path := s.openFile("导入常用语")
		if path == "" {
			return func() { s.commonBusy = false }
		}
		response, err := s.backend.Call(op("import_common_phrases", "path", path, "replace", replace))
		if err != nil {
			return func() { s.commonBusy = false; s.report, s.reportErrors = "导入失败："+err.Error(), nil }
		}
		list, err := s.backend.Call(op("list_common_phrases"))
		return func() {
			s.commonBusy = false
			if err != nil {
				s.report, s.reportErrors = "导入完成，但刷新失败："+err.Error(), nil
				return
			}
			s.common = list.CommonPhrases
			s.report, s.reportErrors = describe(response.Report, "常用语新增")
		}
	})
}
