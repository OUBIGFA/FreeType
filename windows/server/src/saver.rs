//! 后台写盘队列。同一路径尚未开始的工作用新快照替换；停机前排空队列。
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

pub(crate) type Job = Box<dyn FnOnce() -> Result<(), String> + Send>;

#[derive(Default)]
struct State {
    queue: BTreeMap<&'static str, Job>,
    running: bool,
    stop: bool,
    error: Option<String>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

#[derive(Default)]
pub(crate) struct Saver {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Saver {
    pub fn enqueue(&mut self, slot: &'static str, job: Job) -> Result<(), String> {
        if self.worker.is_none() {
            let shared = self.shared.clone();
            self.worker = Some(
                std::thread::Builder::new()
                    .name("freetype-save".into())
                    .spawn(move || run(&shared))
                    .map_err(|error| format!("无法启动保存线程：{error}"))?,
            );
        }
        self.shared.state.lock().unwrap().queue.insert(slot, job);
        self.shared.ready.notify_all();
        Ok(())
    }

    /// 主循环取失败结果以便重试；换词库或退出时可以等到所有在途快照完成。
    pub fn result(&self, wait: bool) -> Result<(), String> {
        let mut state = self.shared.state.lock().unwrap();
        if wait {
            state = self
                .shared
                .ready
                .wait_while(state, |state| state.running || !state.queue.is_empty())
                .unwrap();
        }
        state.error.take().map_or(Ok(()), Err)
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.shared.state.lock().unwrap().stop = true;
            self.shared.ready.notify_all();
            let _ = worker.join();
        }
    }
}

fn run(shared: &Shared) {
    crate::background_thread();
    loop {
        let job = {
            let state = shared.state.lock().unwrap();
            let mut state = shared
                .ready
                .wait_while(state, |s| s.queue.is_empty() && !s.stop)
                .unwrap();
            let Some((_, job)) = state.queue.pop_first() else {
                return;
            };
            state.running = true;
            job
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
            .unwrap_or_else(|_| Err("后台保存发生内部错误".into()));
        let mut state = shared.state.lock().unwrap();
        state.running = false;
        if let Err(error) = result {
            state.error = Some(error);
        }
        shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;

    #[test]
    fn coalesces_pending_jobs_and_drains_before_exit() {
        let mut saver = Saver::default();
        let (ready_tx, ready_rx) = channel();
        let (go_tx, go_rx) = channel();
        saver
            .enqueue(
                "gate",
                Box::new(move || {
                    ready_tx.send(()).unwrap();
                    go_rx.recv().unwrap();
                    Ok(())
                }),
            )
            .unwrap();
        ready_rx.recv().unwrap();
        let (tx, rx) = channel();
        for n in 1..=3 {
            let tx = tx.clone();
            saver
                .enqueue(
                    "same",
                    Box::new(move || {
                        tx.send(n).unwrap();
                        Ok(())
                    }),
                )
                .unwrap();
        }
        go_tx.send(()).unwrap();
        saver.result(true).unwrap();
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [3]);
    }
}
