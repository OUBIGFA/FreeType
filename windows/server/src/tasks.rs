//! 后台任务线程：同步快照的比较、合并、写文件，以及同步记录登记。
//!
//! 按键和这些工作原来排在同一个引擎线程里，一次同步要占几百毫秒，开机时可达数秒，
//! 期间按键只能等。现在引擎线程只取快照（毫秒级），重计算放到这里，按提交顺序逐个执行；
//! 线程优先级低于输入路径（见 [`crate::background_thread`]）。任务只带数据快照和路径，
//! 不引用引擎，引擎被换掉也不受影响。
//!
//! 需要改动引擎或用户文件的结果交回引擎线程应用（[`Step::Apply`]）。应用期间本线程等待，
//! 不取下一项，所以同步记录文件始终只有一处在读写，顺序与提交顺序一致。

use std::cell::OnceCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use freetype_protocol::Response;

use crate::host::Host;

/// 在后台线程执行的工作。
pub(crate) type Job = Box<dyn FnOnce() -> Step + Send>;

/// 交回引擎线程执行的闭包，由主循环调用。
pub type Apply = Box<dyn FnOnce(&mut Host) + Send>;

/// 后台工作做完后的去向。
pub(crate) enum Step {
    /// 直接应答（导出、预览）。
    Reply(Response),
    /// 回到引擎线程应用结果后再应答。
    Apply(Box<dyn FnOnce(&mut Host) -> Response + Send>),
    /// 没有应答对象（同步记录登记）。
    Done,
}

struct Queued {
    job: Job,
    reply: Option<Sender<Response>>,
}

/// 已提交未做完的数量；`applying` 表示后台线程正等着引擎线程应用结果。
#[derive(Default)]
struct Pending {
    state: Mutex<(usize, bool)>,
    idle: Condvar,
}

#[derive(Default)]
pub(crate) struct Tasks {
    queue: OnceCell<Sender<Queued>>,
    worker: OnceCell<JoinHandle<()>>,
    pending: Arc<Pending>,
    /// 主循环收 [`Apply`] 的通道；没有主循环（单元测试）时为空，只能在引擎线程内联执行。
    apply: Option<Sender<Apply>>,
}

impl Tasks {
    pub fn set_apply_channel(&mut self, apply: Sender<Apply>) {
        self.apply = Some(apply);
    }

    pub fn submit(&self, job: Job, reply: Option<Sender<Response>>) {
        self.pending.state.lock().unwrap().0 += 1;
        let queue = self.queue.get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            let (pending, apply) = (self.pending.clone(), self.apply.clone());
            let worker = std::thread::Builder::new()
                .name("freetype-tasks".into())
                .spawn(move || run(rx, &pending, apply))
                .expect("无法启动后台任务线程");
            let _ = self.worker.set(worker);
            tx
        });
        if let Err(mpsc::SendError(queued)) = queue.send(Queued { job, reply }) {
            finish(&self.pending);
            if let Some(reply) = queued.reply { let _ = reply.send(Response::error("后台任务线程已停止")); }
        }
    }

    /// 等已提交的工作做完，在引擎线程按顺序读写同步记录前调用。
    /// 在 [`Apply`] 里调用时立即返回：之前提交的已按顺序做完，之后提交的本来就排在后面，
    /// 而后台线程正等着这次应用，再等就会互相卡住。
    pub fn wait_idle(&self) {
        let state = self.pending.state.lock().unwrap();
        drop(self.pending.idle.wait_while(state, |(count, applying)| !*applying && *count > 0).unwrap());
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        // 关闭队列后后台线程做完已排队的工作再退出；停机时主循环先丢弃未处理的 Apply，不会互等。
        self.queue.take();
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn finish(pending: &Pending) {
    let mut state = pending.state.lock().unwrap();
    state.0 = state.0.saturating_sub(1);
    pending.idle.notify_all();
}

fn set_applying(pending: &Pending, applying: bool) {
    pending.state.lock().unwrap().1 = applying;
}

fn run(queue: Receiver<Queued>, pending: &Pending, apply: Option<Sender<Apply>>) {
    crate::background_thread();
    for Queued { job, reply } in queue {
        let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
            .unwrap_or_else(|_| Step::Reply(Response::error("后台任务内部出错")));
        let response = match step {
            Step::Reply(response) => Some(response),
            Step::Done => None,
            Step::Apply(work) => {
                let (done, wait) = mpsc::channel();
                set_applying(pending, true);
                let sent = apply.as_ref().is_some_and(|apply| apply.send(Box::new(move |host: &mut Host| {
                    let _ = done.send(work(host));
                })).is_ok());
                // 主循环退出时丢弃闭包，接收端随即返回错误，不会一直等。
                let response = if sent { wait.recv().ok() } else { Some(Response::error("服务正在退出，后台结果没有应用")) };
                set_applying(pending, false);
                response
            }
        };
        if let (Some(reply), Some(response)) = (reply, response) { let _ = reply.send(response); }
        finish(pending);
    }
}
