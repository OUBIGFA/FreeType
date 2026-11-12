//! 邮箱、网址补全（照搜狗输入法）：编辑区里是原样的英文，候选是补全以后的写法。
//!
//! - 打 `@`：候选是常用邮箱后缀，`abc@` 出 `abc@qq.com`、`abc@gmail.com`……，接着打 `@g` 只留 g 开头的；
//! - 打 `gmail.`、`www.baidu.`：候选是 `.com`、`.cn` 这些结尾；`www.baidu.c` 只留 c 开头的；
//! - 第一个候选是原样的输入，空格选光标处的候选，回车原样上屏。数字在这里是正文（邮箱里常有），
//!   不拿来选词，用方向键或鼠标选。
//!
//! 什么时候进来由 [`crate::input`] 决定：打 `@`；打到一半按小键盘的点；打的是常见网站名、
//! 英文单词、www 开头时按句号。

/// 常用邮箱后缀（国内、国际常见的在前）。
pub const EMAIL_DOMAINS: [&str; 16] = [
    "qq.com",
    "163.com",
    "gmail.com",
    "outlook.com",
    "126.com",
    "foxmail.com",
    "hotmail.com",
    "icloud.com",
    "sina.com",
    "yahoo.com",
    "139.com",
    "aliyun.com",
    "sohu.com",
    "live.com",
    "yeah.net",
    "protonmail.com",
];

/// 网址、邮箱结尾。
pub const TLDS: [&str; 10] = ["com", "cn", "net", "org", "com.cn", "io", "co", "edu.cn", "gov.cn", "top"];

/// 按句号就进补全的名字（打完这些再按句号多半是在写网址、邮箱，不是句末）。
const SITE_NAMES: [&str; 40] = [
    "www", "gmail", "outlook", "hotmail", "qq", "163", "126", "yahoo", "icloud", "foxmail", "sina", "sohu",
    "aliyun", "github", "gitlab", "google", "baidu", "taobao", "tmall", "jd", "bilibili", "zhihu", "weibo",
    "youtube", "twitter", "facebook", "microsoft", "apple", "amazon", "live", "msn", "csdn", "douyin",
    "wikipedia", "reddit", "openai", "chatgpt", "mail", "http", "https",
];

/// 这串字母后面按句号要不要进补全。`english`：引擎把它当成英文单词（首选就是它）。
pub fn wants_completion(input: &str, english: bool) -> bool {
    let lower = input.to_ascii_lowercase();
    !lower.is_empty()
        && lower.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && (english || SITE_NAMES.contains(&lower.as_str()) || lower.starts_with("www"))
}

/// 能进编辑区的字符：字母、数字和网址、邮箱里常见的符号。
pub fn accepts(c: char) -> bool {
    c.is_ascii_alphanumeric() || "@.-_+/:?=&#%~".contains(c)
}

/// 编辑区。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Raw {
    pub text: String,
    /// 光标（全部候选里的序号）。
    pub cursor: usize,
}

impl Raw {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            cursor: 0,
        }
    }

    pub fn push(&mut self, c: char) {
        self.text.push(c);
        self.cursor = 0;
    }

    /// 删最后一个字符；删空了返回 false。
    pub fn pop(&mut self) -> bool {
        self.text.pop();
        self.cursor = 0;
        !self.text.is_empty()
    }

    /// 候选：原样的输入在前，补全的在后。
    pub fn candidates(&self) -> Vec<String> {
        let mut out = vec![self.text.clone()];
        for s in suggestions(&self.text) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    pub fn current(&self) -> String {
        let all = self.candidates();
        all.get(self.cursor.min(all.len() - 1)).cloned().unwrap_or_default()
    }

    pub fn move_cursor(&mut self, prev: bool) {
        let n = self.candidates().len();
        self.cursor = if prev {
            (self.cursor + n - 1) % n
        } else {
            (self.cursor + 1) % n
        };
    }
}

/// 补全的写法（不含原样的输入）。
pub fn suggestions(text: &str) -> Vec<String> {
    if let Some(at) = text.rfind('@') {
        let (local, domain) = (&text[..at], &text[at + 1..]);
        if domain.contains('.') {
            return complete_tld(text);
        }
        return EMAIL_DOMAINS
            .iter()
            .filter(|d| d.starts_with(domain) && **d != domain)
            .map(|d| format!("{local}@{d}"))
            .collect();
    }
    if text.contains('.') {
        return complete_tld(text);
    }
    Vec::new()
}

/// 最后一个点后面补上常见结尾：`gmail.` → gmail.com，`www.baidu.c` → www.baidu.com、www.baidu.cn。
fn complete_tld(text: &str) -> Vec<String> {
    let Some(dot) = text.rfind('.') else {
        return Vec::new();
    };
    let (head, tail) = (&text[..dot], &text[dot + 1..]);
    if head.is_empty() || head.ends_with('@') || TLDS.contains(&tail) {
        return Vec::new();
    }
    TLDS.iter()
        .filter(|t| t.starts_with(tail) && **t != tail)
        .map(|t| format!("{head}.{t}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_lists_mail_domains_and_filters() {
        let raw = Raw::new("@");
        let all = raw.candidates();
        assert_eq!(all[0], "@");
        assert_eq!(all[1], "@qq.com");
        assert!(all.contains(&"@gmail.com".to_string()));
        assert_eq!(all.len(), 1 + EMAIL_DOMAINS.len());

        let all = Raw::new("abc@g").candidates();
        assert_eq!(all, vec!["abc@g", "abc@gmail.com"]);
        assert_eq!(Raw::new("abc@qq.com").candidates(), vec!["abc@qq.com"], "写完了就不补");
    }

    #[test]
    fn dot_completes_tld() {
        let all = Raw::new("gmail.").candidates();
        assert_eq!(all[0], "gmail.");
        assert_eq!(all[1], "gmail.com");
        assert_eq!(
            Raw::new("www.baidu.c").candidates(),
            vec!["www.baidu.c", "www.baidu.com", "www.baidu.cn", "www.baidu.com.cn", "www.baidu.co"]
        );
        assert_eq!(Raw::new("a@company.").candidates()[1], "a@company.com");
        assert_eq!(Raw::new(".").candidates(), vec!["."]);
    }

    #[test]
    fn cursor_wraps_and_resets() {
        let mut raw = Raw::new("x@");
        raw.move_cursor(true);
        assert_eq!(raw.current(), format!("x@{}", EMAIL_DOMAINS[EMAIL_DOMAINS.len() - 1]));
        raw.move_cursor(false);
        assert_eq!(raw.current(), "x@");
        raw.push('q');
        assert_eq!(raw.cursor, 0);
        assert!(raw.pop());
        assert!(raw.pop());
        assert!(!raw.pop());
    }

    #[test]
    fn completion_trigger() {
        assert!(wants_completion("gmail", false));
        assert!(wants_completion("WWW", false));
        assert!(wants_completion("hello", true));
        assert!(!wants_completion("nihao", false));
        assert!(!wants_completion("", true));
        assert!(accepts('@') && accepts('7') && !accepts(','));
    }
}
