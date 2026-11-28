//! 英文/拆字共用的只读查询表。词条正文留在映射文件中，仅为本次候选分配字符串。
//! 格式由 tools/make_input_tools.py 生成；全部边界、排序和校验在加载时验证一次。
use anyhow::{bail, ensure, Context, Result};
use memmap2::Mmap;
use std::{cmp::Reverse, collections::HashSet, fs::File, path::Path};

enum Storage {
    Mapped(Mmap),
    Owned(Vec<u8>),
}
impl Storage {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Mapped(data) => data,
            Self::Owned(data) => data,
        }
    }
}

pub struct LookupTable {
    data: Storage,
    count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupEntry {
    pub key: String,
    pub text: String,
    pub reading: String,
    pub gloss: String,
    pub rank: u32,
}

struct Record<'a> {
    key: &'a str,
    rank: u32,
    text: &'a str,
    reading: &'a str,
    gloss: &'a str,
}

impl LookupTable {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path.as_ref())
            .with_context(|| format!("读取查询词典 {}", path.as_ref().display()))?;
        // 安装资源只读且由升级流程在进程退出后替换，不在映射存活期间原地修改。
        Self::validated(Storage::Mapped(unsafe { Mmap::map(&file)? }))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::validated(Storage::Owned(bytes))
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn validated(data: Storage) -> Result<Self> {
        let bytes = data.bytes();
        ensure!(
            bytes.len() >= 24 && &bytes[..8] == b"FTLOOK01",
            "不支持的查询词典格式"
        );
        let count = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
        ensure!(count <= 5_000_000, "查询词典条数异常");
        let header = 16 + (count + 1) * 8;
        ensure!(header <= bytes.len(), "查询词典索引不完整");
        ensure!(
            crc32fast::hash(&bytes[16..]) == u32::from_le_bytes(bytes[12..16].try_into()?),
            "查询词典校验失败"
        );
        let table = Self { data, count };
        ensure!(
            table.offset(0) == header && table.offset(count) == table.data.bytes().len(),
            "查询词典边界错误"
        );
        let mut previous = "";
        for index in 0..count {
            ensure!(
                table.offset(index) < table.offset(index + 1)
                    && table.offset(index + 1) <= table.data.bytes().len(),
                "查询词典索引越界"
            );
            let row = table.checked_record(index)?;
            ensure!(
                !row.key.is_empty()
                    && row.key.bytes().all(|c| c.is_ascii_lowercase())
                    && row.key >= previous,
                "查询词典编码或排序错误"
            );
            ensure!(
                !row.text.is_empty() && !row.text.contains('\0'),
                "查询词典词条为空或损坏"
            );
            previous = row.key;
        }
        Ok(table)
    }

    fn offset(&self, index: usize) -> usize {
        let start = 16 + index * 8;
        u64::from_le_bytes(self.data.bytes()[start..start + 8].try_into().unwrap()) as usize
    }

    fn checked_record(&self, index: usize) -> Result<Record<'_>> {
        let text =
            std::str::from_utf8(&self.data.bytes()[self.offset(index)..self.offset(index + 1)])?;
        let Some(text) = text.strip_suffix('\n') else {
            bail!("查询词典记录未结束");
        };
        let fields: Vec<_> = text.split('\t').collect();
        ensure!(fields.len() == 5, "查询词典记录列数错误");
        Ok(Record {
            key: fields[0],
            rank: fields[1].parse()?,
            text: fields[2],
            reading: fields[3],
            gloss: fields[4],
        })
    }

    fn record(&self, index: usize) -> Record<'_> {
        // 已验证的数据只拆借用切片，避免每个按键为命中区间逐行分配 Vec。
        let text =
            std::str::from_utf8(&self.data.bytes()[self.offset(index)..self.offset(index + 1)])
                .unwrap();
        let mut fields = text.trim_end_matches('\n').split('\t');
        Record {
            key: fields.next().unwrap(),
            rank: fields.next().unwrap().parse().unwrap(),
            text: fields.next().unwrap(),
            reading: fields.next().unwrap(),
            gloss: fields.next().unwrap(),
        }
    }

    fn lower_bound(&self, key: &str) -> usize {
        let (mut start, mut end) = (0, self.count);
        while start < end {
            let middle = start + (end - start) / 2;
            if self.record(middle).key < key {
                start = middle + 1;
            } else {
                end = middle;
            }
        }
        start
    }

    /// 精确编码优先，普通单词在前缀短语之前；同层按源词频与词长排序。
    pub fn search(&self, prefix: &str, limit: usize) -> Vec<LookupEntry> {
        if prefix.is_empty() || !prefix.bytes().all(|c| c.is_ascii_lowercase()) || limit == 0 {
            return Vec::new();
        }
        let start = self.lower_bound(prefix);
        let end = self.lower_bound(&format!("{prefix}{{"));
        let mut matches: Vec<_> = (start..end).map(|index| self.record(index)).collect();
        let priority = |row: &Record<'_>| {
            (
                row.key == prefix,
                !row.text.contains([' ', '-']),
                row.rank,
                Reverse(row.key.len()),
            )
        };
        let order = |a: &Record<'_>, b: &Record<'_>| {
            priority(b)
                .cmp(&priority(a))
                .then_with(|| a.text.cmp(b.text))
        };
        // 单字母前缀可能命中几万条，只为有限候选排序。
        let take = limit.min(1000);
        if matches.len() > take {
            matches.select_nth_unstable_by(take, order);
            matches.truncate(take);
        }
        matches.sort_unstable_by(order);
        let mut seen = HashSet::new();
        matches
            .into_iter()
            .filter(|row| seen.insert(row.text))
            .take(limit.min(1000))
            .map(|row| LookupEntry {
                key: row.key.into(),
                text: row.text.into(),
                reading: row.reading.into(),
                gloss: row.gloss.into(),
                rank: row.rank,
            })
            .collect()
    }

    pub fn exact(&self, key: &str) -> Option<LookupEntry> {
        let index = self.lower_bound(key);
        if index == self.count {
            return None;
        }
        let row = self.record(index);
        (row.key == key).then(|| LookupEntry {
            key: row.key.into(),
            text: row.text.into(),
            reading: row.reading.into(),
            gloss: row.gloss.into(),
            rank: row.rank,
        })
    }
}
