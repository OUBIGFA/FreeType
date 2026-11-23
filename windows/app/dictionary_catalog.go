package main

import (
	"fmt"
	"github.com/egoist/mygo/ui"
	"strings"
)

const dictionaryPageSize = 50

// 查询使用版本号丢弃较早的应答，搜索或切换来源时不显示上一次的结果。
func (s *settings) queryDictionary(offset int) {
	s.dictionarySequence++
	sequence, name, query := s.dictionarySequence, s.dictionaryName, s.dictionaryQuery
	s.dictionaryBusy = true
	s.dictionaryPage = nil
	s.background(func() func() {
		response, err := s.backend.Call(op("query_dictionary", "name", name, "query", query, "offset", offset, "limit", dictionaryPageSize))
		return func() {
			if s.dictionarySequence != sequence {
				return
			}
			s.dictionaryBusy = false
			if err != nil {
				s.notice = "读取词条失败：" + err.Error()
				return
			}
			s.dictionaryPage = response.DictionaryPage
			s.dictionarySelected = map[string]bool{}
		}
	})
}

func (s *settings) changeDictionary(dict Dictionary, name string, enabled bool, description string) {
	s.dictionaryBusy = true
	s.background(func() func() {
		response, err := s.backend.Call(op("update_dictionary", "name", dict.Name, "new_name", strings.TrimSpace(name), "enabled", enabled, "description", description))
		return func() {
			s.dictionaryBusy = false
			if err != nil {
				s.notice = "保存词库失败：" + err.Error()
				return
			}
			s.dicts = response.Dictionaries
			s.notice = "词库设置已保存"
			if s.dictionaryName == dict.Name {
				s.dictionaryName, s.dictionaryRename = strings.TrimSpace(name), strings.TrimSpace(name)
				s.queryDictionary(0)
			}
		}
	})
}

func dictionaryEntryKey(entry DictionaryEntry) string { return entry.Pinyin + "\t" + entry.Word }

func (s *settings) deleteSelectedDictionaryEntries() {
	if s.dictionaryPage == nil {
		return
	}
	name := s.dictionaryName
	var entries []DictionaryEntry
	for _, entry := range s.dictionaryPage.Entries {
		if s.dictionarySelected[dictionaryEntryKey(entry)] {
			entries = append(entries, entry)
		}
	}
	if len(entries) == 0 {
		return
	}
	s.dictionaryBusy = true
	s.background(func() func() {
		_, err := s.backend.Call(op("delete_dictionary_entries", "name", name, "entries", entries))
		list, listErr := s.backend.Call(op("list_dictionaries"))
		return func() {
			s.dictionaryBusy = false
			if err != nil {
				s.notice = "删除词条失败：" + err.Error()
				return
			}
			s.notice = fmt.Sprintf("已删除 %d 条选中词条", len(entries))
			if listErr == nil {
				s.dicts = list.Dictionaries
			} else {
				s.notice += "；刷新列表失败：" + listErr.Error()
			}
			if name == s.dictionaryName {
				s.queryDictionary(0)
			}
		}
	})
}

func (s *settings) dictionaryBrowser(c *ui.Context) {
	p := colors(c)
	section(c, "浏览导入词库", func() {
		s.settingBlock(c, "dicts/词库词条", func() {
			if s.dictionaryName == "" {
				row(c, "未选择词库", "在上方点击已导入词库的「浏览」查看词条", nil)
				return
			}
			page := s.dictionaryPage
			ui.Column(c).FillWidth().Gap(1).Dividers(1, p.border).Children(func() {
				// 词库名称与说明编辑区
				ui.Column(c).FillWidth().Padding(12, 0, 14, 0).Gap(10).Children(func() {
					ui.Row(c).FillWidth().Gap(12).Wrap().AlignItems(ui.End).Children(func() {
						ui.Field(c, "词库名称", func() {
							ui.TextInput(c, &s.dictionaryRename).Label("词库名称").FillWidth()
						}).Width(200).MaxWidthPercent(100)
						ui.Field(c, "词库说明", func() {
							ui.TextInput(c, &s.dictionaryDescription).Label("词库说明").Placeholder("说明用途或来源，最多 500 字").FillWidth()
						}).Grow(1).Basis(220).MinWidth(180)
						if ui.Button(c, "保存名称与说明").Disabled(s.dictionaryBusy || page == nil).Clicked() {
							s.changeDictionary(page.Dictionary, s.dictionaryRename, page.Dictionary.Enabled, s.dictionaryDescription)
						}
					})
					if page != nil && page.Dictionary.Source != "" {
						ui.Text(c, "导入来源："+page.Dictionary.Source).FontSize(textSize(c, 12.5)).TextColor(p.muted).LineHeight(1.4).Wrap()
					}
				})
				// 搜索、翻页与批量操作区
				ui.Column(c).FillWidth().Padding(12, 0).Gap(10).Children(func() {
					ui.Row(c).FillWidth().Gap(12).Wrap().AlignItems(ui.Center).Children(func() {
						if ui.SearchField(c, &s.dictionaryQuery).Label("搜索词库词条").Placeholder("搜索词语或拼音").
							Grow(1).Basis(220).MinWidth(180).MaxWidthPercent(100).Changed() {
							s.queryDictionary(0)
						}
						if page != nil {
							totalPages := max(1, (page.Total+dictionaryPageSize-1)/dictionaryPageSize)
							currentPage := page.Offset/dictionaryPageSize + 1
							ui.Row(c).Gap(8).Wrap().AlignItems(ui.Center).Children(func() {
								ui.Text(c, fmt.Sprintf("共 %d 条 · 第 %d / %d 页", page.Total, currentPage, totalPages)).
									FontSize(textSize(c, 13)).TextColor(p.muted).FontFeatures("tnum")
								if ui.Button(c, "上一页").Label("词库上一页").Disabled(s.dictionaryBusy || page.Offset == 0).Clicked() {
									s.queryDictionary(max(0, page.Offset-dictionaryPageSize))
								}
								if ui.Button(c, "下一页").Label("词库下一页").Disabled(s.dictionaryBusy || page.Offset+len(page.Entries) >= page.Total).Clicked() {
									s.queryDictionary(page.Offset + dictionaryPageSize)
								}
							})
						}
					})
					if page != nil && len(page.Entries) > 0 {
						all, count := true, 0
						for _, entry := range page.Entries {
							checked := s.dictionarySelected[dictionaryEntryKey(entry)]
							all = all && checked
							if checked {
								count++
							}
						}
						ui.Row(c).FillWidth().Gap(12).Wrap().AlignItems(ui.Center).Justify(ui.SpaceBetween).Children(func() {
							ui.Row(c).Gap(12).Wrap().AlignItems(ui.Center).Children(func() {
								if checkbox(c, &all, "本页全选").Disabled(s.dictionaryBusy).Changed() {
									for _, entry := range page.Entries {
										s.dictionarySelected[dictionaryEntryKey(entry)] = all
									}
								}
								if count > 0 {
									ui.Text(c, fmt.Sprintf("已选 %d 条", count)).FontSize(textSize(c, 13)).TextColor(p.muted).FontFeatures("tnum")
								}
							})
							if ui.Button(c, "删除选中").Label("删除选中词库词条").Disabled(s.dictionaryBusy || count == 0).Clicked() {
								s.confirmDictionaryDelete = true
							}
						})
					}
				})
				// 词条列表或状态提示
				if page == nil {
					if s.dictionaryBusy {
						row(c, "正在读取词条", "", nil)
					} else {
						row(c, "词条尚未载入", "读取失败或尚未完成，可点击重试", func() {
							if ui.Button(c, "重试读取词条").Clicked() {
								s.queryDictionary(0)
							}
						})
					}
					return
				}
				if len(page.Entries) == 0 {
					title := "这份词库里没有词条"
					if strings.TrimSpace(s.dictionaryQuery) != "" {
						title = "没有匹配的词条"
					}
					row(c, title, "", nil)
					return
				}
				ui.List(c, nil, len(page.Entries), func(i int) {
					entry := page.Entries[i]
					key := dictionaryEntryKey(entry)
					checked := s.dictionarySelected[key]
					item := ui.Row(c).Padding(8, 0).MinHeight(46).Gap(12).AlignItems(ui.Center)
					if item.Hovered() {
						item.Background(p.cardHover)
					}
					item.Children(func() {
						if checkbox(c, &checked, "").Label("选择 " + entry.Word + " " + entry.Pinyin).Disabled(s.dictionaryBusy).Changed() {
							s.dictionarySelected[key] = checked
						}
						ui.Text(c, entry.Word).FontWeight(500).Width(132).SingleLine().Tooltip(entry.Word)
						ui.Text(c, entry.Pinyin).Grow(1).Basis(0).MinWidth(0).SingleLine().TextColor(p.muted).Tooltip(entry.Pinyin)
						ui.Textf(c, "权重 %d", entry.Weight).FontSize(textSize(c, 13)).TextColor(p.muted).MinWidth(textSize(c, 84)).SingleLine().Shrink(0).TextAlign(ui.End).FontFeatures("tnum")
					})
				}).MaxHeight(380)
			})
		})
	})
	switch ui.AlertDialog(c, &s.confirmDictionaryDelete, "删除选中词条", "只从当前导入词库删除；其他来源和个人词不变；可用备份恢复", "取消", "删除") {
	case 1:
		s.deleteSelectedDictionaryEntries()
	}
}
