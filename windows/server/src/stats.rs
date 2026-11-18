//! Opt-in local-only statistics. No text, app identifiers, or individual events are persisted.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use freetype_protocol::TypingDay;
use wowtype_core::autospace::is_han;

type Days = BTreeMap<String, TypingDay>;

pub(crate) struct Stats {
    path: PathBuf,
    days: Option<Days>,
    last_commit: Option<(u64, String, Instant)>,
    dirty: bool,
    saver: crate::saver::Saver,
}

impl Stats {
    pub fn new(directory: PathBuf) -> Self {
        Self { path: directory.join("typing-stats.json"), days: None, last_commit: None, dirty: false, saver: Default::default() }
    }

    fn load(&mut self) -> Result<&mut Days, String> {
        if self.days.is_none() {
            let days = match std::fs::read(&self.path) {
                Ok(data) => serde_json::from_slice(&data).map_err(|e| format!("统计文件无法读取：{e}"))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Days::new(),
                Err(error) => return Err(format!("读取统计失败：{error}")),
            };
            self.days = Some(days);
        }
        Ok(self.days.as_mut().unwrap())
    }

    pub fn prepare(&mut self) {
        if let Err(error) = self.load() { crate::log(&error); }
    }

    pub fn break_activity(&mut self) { self.last_commit = None; }

    pub fn record(&mut self, client: u64, scan: Option<u16>, commit: Option<&str>, rank: usize, keep: u32) {
        if scan.is_none() && commit.is_none() { return; }
        let Some((date, hour)) = today() else { return; };
        let now = Instant::now();
        let new_day = self.days.as_ref().is_none_or(|days| !days.contains_key(&date));
        let activity = if commit.is_some() {
            self.last_commit.replace((client, date.clone(), now)).and_then(|(id, old_date, time)| {
                (id == client && old_date == date).then_some(now.saturating_duration_since(time).as_millis() as u64)
            }).filter(|delta| *delta <= 10_000).unwrap_or(0)
        } else { 0 };
        // Enabling statistics preloads the document outside the key path. A failed load
        // stays inactive until the user clears the damaged file or restarts the service.
        let Some(days) = self.days.as_mut() else { return; };
        if new_day { retain(days, &date, keep); }
        let day = days.entry(date).or_default();
        if let Some(scan) = scan.filter(|scan| (1..=0x1ff).contains(scan)) {
            increment(day.keys.entry(scan).or_default(), 1);
        }
        if let Some(text) = commit {
            increment(&mut day.active_ms, activity);
            if rank > 0 { increment(&mut day.candidates[(rank - 1).min(10)], 1); }
            for ch in text.chars().filter(|ch| !ch.is_whitespace() && !ch.is_control()) {
                let kind = if is_han(ch) { 0 } else if ch.is_ascii_alphabetic() { 1 }
                    else if ch.is_numeric() { 2 } else if ch.is_ascii_punctuation()
                        || ('\u{3001}'..='\u{303f}').contains(&ch) || "，。？！、：；（）【】《》「」‘’“”…—".contains(ch) { 3 } else { 4 };
                increment(&mut day.characters[kind], 1);
                increment(&mut day.hours[hour], 1);
            }
        }
        self.dirty = true;
    }

    pub fn snapshot(&mut self, keep: u32) -> Result<Days, String> {
        let mut days = self.load()?.clone();
        if let Some((date, _)) = today() { retain(&mut days, &date, keep); }
        Ok(days)
    }

    pub fn save(&mut self, wait: bool) -> Result<(), String> {
        if self.saver.result(false).is_err() { self.dirty = true; }
        if self.dirty {
            let path = self.path.clone();
            let days = self.load()?.clone();
            self.saver.enqueue("stats", Box::new(move || {
                let text = serde_json::to_string(&days).map_err(|err| err.to_string())?;
                crate::store::write_atomic(&path, &text).map_err(|err| format!("保存输入统计失败：{err}"))
            }))?;
            self.dirty = false;
        }
        if wait { self.saver.result(true)?; }
        Ok(())
    }

    pub fn clear(&mut self) -> Result<(), String> {
        // Host serializes requests. Drain old jobs before deleting so no snapshot can resurrect data.
        let _ = self.saver.result(true);
        match std::fs::remove_file(&self.path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(format!("清空输入统计失败：{error}")),
        }
        self.days = Some(Days::new());
        self.break_activity();
        self.dirty = false;
        Ok(())
    }
}

fn increment(slot: &mut u64, value: u64) { *slot = slot.saturating_add(value); }

#[cfg(windows)]
fn today() -> Option<(String, usize)> {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let time = unsafe { GetLocalTime() };
    Some((format!("{:04}-{:02}-{:02}", time.wYear, time.wMonth, time.wDay), usize::from(time.wHour)))
}

#[cfg(not(windows))]
fn today() -> Option<(String, usize)> { None }

fn civil_day(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i64>().ok().filter(|y| (1..=9999).contains(y))?;
    let month = parts.next()?.parse::<usize>().ok().filter(|m| (1..=12).contains(m))?;
    let day = parts.next()?.parse::<i64>().ok().filter(|d| (1..=31).contains(d))?;
    let previous = year - 1;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let offsets = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    Some(365 * previous + previous / 4 - previous / 100 + previous / 400 + offsets[month - 1] + i64::from(month > 2 && leap) + day)
}

fn retain(days: &mut Days, date: &str, keep: u32) {
    if keep == 0 { return; }
        // Current day is validated by Windows; reject an unrecognized date rather than pruning.
        let Some(today) = civil_day(date) else { return; };
        let limit = today - i64::from(keep) + 1;
    days.retain(|date, _| civil_day(date).is_some_and(|day| day >= limit));
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn retains_local_calendar_days() {
        let mut days = ["2024-02-28", "2024-02-29", "2024-03-01"].map(|day| (day.to_owned(), TypingDay::default())).into();
        retain(&mut days, "2024-03-01", 2);
        assert_eq!(days.keys().map(String::as_str).collect::<Vec<_>>(), ["2024-02-29", "2024-03-01"]);
    }

    #[test]
    fn records_numbers_and_does_not_restore_cleared_snapshots() {
        let root = std::env::var_os("FREETYPE_TEST_TEMP").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_temp"));
        let mut stats = Stats::new(root.join(format!("typing-stats-{}", std::process::id())));
        stats.prepare();
        stats.record(1, Some(0x10), Some("你好 Test 42！"), 11, 365);
        let days = stats.snapshot(365).unwrap();
        let day = days.values().next().unwrap();
        assert_eq!(day.characters, [2, 4, 2, 1, 0]);
        assert_eq!(day.candidates[10], 1);
        assert_eq!(day.keys.get(&0x10), Some(&1));
        stats.save(false).unwrap();
        stats.clear().unwrap();
        assert!(!stats.path.exists());
        assert!(stats.snapshot(365).unwrap().is_empty());
        std::fs::remove_dir_all(stats.path.parent().unwrap()).unwrap();
    }
}
