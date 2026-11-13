<div align="center">

<img src="dist/FreeType_logo.png" width="160" alt="FreeType logo">

# FreeType

A local Windows Chinese input method. Your input stays on your computer.

[简体中文](README.md) | English

![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-555555)

</div>

## What FreeType is

FreeType is a local Chinese input method. The input engine, dictionaries, and learning data run on your computer. Normal Chinese input needs no network. FreeType does not send the text that you type to a server.

## Software preview

These images come from the FreeType installation on this computer. The General page is first.

<table>
<tr>
<td><img src="windows/assets/readme/settings-general-installed.png" width="420" alt="FreeType General settings page"><br>General</td>
<td><img src="windows/assets/readme/settings-typing-installed.png" width="420" alt="FreeType Pinyin and Keys settings page"><br>Pinyin and Keys</td>
</tr>
<tr>
<td><img src="windows/assets/readme/settings-features-installed.png" width="420" alt="FreeType Useful Features settings page"><br>Useful Features</td>
<td><img src="windows/assets/readme/settings-dicts-installed.png" width="420" alt="FreeType Dictionaries settings page"><br>Dictionaries</td>
</tr>
</table>

### Candidate window colors

The candidate window has four selected-candidate styles:

<table>
<tr>
<td><img src="windows/assets/readme/candidate-pink.png" width="260" alt="Pink candidate window"><br>Pink (default)</td>
<td><img src="windows/assets/readme/candidate-blue.png" width="260" alt="Blue candidate window"><br>Blue</td>
</tr>
<tr>
<td><img src="windows/assets/readme/candidate-green.png" width="260" alt="Green candidate window"><br>WeChat green</td>
<td><img src="windows/assets/readme/candidate-sogou.png" width="260" alt="Sogou-style candidate window"><br>Sogou style (no color block)</td>
</tr>
</table>

## Why use FreeType

- **Local input. Your data stays on your computer.** The input engine, dictionaries, and learning data run locally. Normal Chinese input uses no network and sends no typed text to a server.
- **It learns your choices.** Your selected words and word pairs affect the candidate order.
- **It handles typing mistakes.** It can find the right word when nearby keys are pressed or two keys are reversed.
- **It moves your dictionaries with you.** It can import dictionaries and phrases from Shuishan, Sogou, Rime, and WowType.
- **It gives you input choices.** Use full pinyin, seven double-pinyin schemes, fuzzy pinyin, and several page keys.
- **It makes mixed text easier.** Use Chinese punctuation, paired punctuation, and spaces between Chinese and Latin text.
- **It keeps useful tools close.** Use quick phrases, symbols, emoji, date and time, and a calculator while you type.
- **It lets you set the candidate window.** Change its font, size, layout, number of candidates, and selected style.
- **It keeps your data on your computer.** Dictionaries, learning data, and settings use local files.

## Main features

### Pinyin that fits your habits

- Full pinyin input.
- Double pinyin: Xiaohe, Ziranma, Microsoft, Sogou, ABC, Ziguang, and PinyinJiajia.
- Fuzzy pinyin groups. For example, treat `z` and `zh` as similar sounds.
- Typo correction. For example, `weixni` can find “微信”.
- Full sentence candidates and word candidates.
- Candidate order based on the dictionary and your learning data.
- An option for more rare characters.

### Chinese, English, and punctuation

- Press Shift once to switch between Chinese and English.
- Use both Shift keys or one Shift key.
- Use one state in all applications, or let each application remember its own state.
- Use Chinese punctuation in Chinese mode.
- Keep a dot half-width after a number, such as `3.14`.
- Type `（`, `“`, `《`, `【`, or `「` to add the matching closing mark.
- Add spaces between Chinese text and Latin text or numbers.

### Toolbar and quick input

In full pinyin mode, type `v` and press Space to open the toolbar. It contains:

- Clipboard history
- Quick phrases
- Symbols
- Emoji

You can also use these features:

- Type `riqi`, `shijian`, `xingqi`, `rq`, or `sj` to see date and time.
- Type `v1+2*3` to calculate an expression.
- Type `v1314` to see Chinese numbers and a formal amount.
- Type `@` to complete common email suffixes.
- Type a site name and a period to complete `.com`, `.cn`, and other domain suffixes.

### Set the candidate window

In Settings, you can change:

- Candidate font.
- Font size.
- Horizontal or vertical layout.
- Number of candidates on each page.
- Color and style for the selected candidate.
- Position of the input hint.

### Dictionaries and quick phrases

FreeType can import:

- Shuishan dictionaries and quick phrases.
- Sogou Cell dictionaries (`.scel`).
- Sogou text dictionaries.
- Rime dictionaries and quick phrases.
- WowType dictionaries.
- Text lists that contain words only.
- Sogou custom phrases.
- WeChat Input quick phrase exports.

Imported dictionaries affect word building and candidate order. They do not replace your learning data.

You can also add a word in Settings. Leave the pinyin field empty and FreeType will try to add the common pronunciation.

### Back up your learning data

The Learned Words page in Settings can:

- Search learned words.
- Delete the memory for one word.
- Add your own words.
- Clear learning data.
- Export a complete learning backup.
- Import words and input habits from a backup.

A complete backup includes words, selection counts, last-use times, and word pairs. Keep the backup in a safe place.

## Data and privacy

- Normal Chinese input runs on your computer.
- Dictionaries, settings, learning data, and quick phrases use local files.
- User data is stored in `%APPDATA%\FreeType`.
- Log files are stored in `%LOCALAPPDATA%\FreeType\logs`.
- Clipboard history is off by default.
- When enabled, clipboard history keeps the latest 20 items in memory.
- A learning backup contains personal input habits. Do not share it in public.

## System requirements

FreeType supports Windows 10/11 x64.

After you install or update FreeType, reopen applications that were already running.

## Common questions

### I can type letters only

1. Press Shift once to switch to Chinese.
2. Check that the input method button shows “中”.
3. Open FreeType Settings and check that the input service is running.

### The candidates are hard to read

Open **FreeType Settings → Candidate Window**. Change the font, font size, layout, or selected style.

### I want one word to appear first

Choose the word more often. FreeType learns your choices. You can also add the word in **Learned Words**.

### I want to use my own dictionary

Open **FreeType Settings → Dictionaries** and select the dictionary file. FreeType shows an import report for Sogou, Rime, and other supported formats.

### I want to save my input habits

Open **FreeType Settings → Learned Words** and click **Export**. After you move to another computer or reinstall FreeType, click **Import**.

## More information

- [Detailed Windows guide](docs/WINDOWS.md)
- [Configuration](docs/CONFIG.md)
- [Input method documentation](docs/README.en.md)

FreeType is based on [WowType](https://github.com/Gloridust/WowType). The Windows settings window and tray program use [mygo](https://github.com/egoist/mygo).
