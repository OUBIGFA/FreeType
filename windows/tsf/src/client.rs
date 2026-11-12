//! 连服务进程的管道客户端。每个文本服务（每个线程）一条连接。
//!
//! 每次请求最多等 [`TIMEOUT_MS`]：服务进程没应答就断开，这个键交还给应用，不让应用卡住。
//! 连不上时按 `HKLM\SOFTWARE\FreeType\ServerPath` 拉起服务进程（隔 [`SPAWN_INTERVAL_MS`] 才再试）。
//! 连接断了（服务进程重启过）时马上重连、把这条请求再发一次，不让这个键漏给应用。
//! 出错都记进 `tsf.log`（见 [`crate::log`]）。

use std::ffi::c_void;

use freetype_protocol::{decode, encode, pipe_name, Request, Response};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_IO_PENDING, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE,
    HANDLE, HLOCAL, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// 一次请求最多等多久。
const TIMEOUT_MS: u32 = 1500;
/// 两次拉起服务进程至少隔多久。
const SPAWN_INTERVAL_MS: u64 = 10_000;

struct Connection {
    pipe: HANDLE,
    event: HANDLE,
    buf: Vec<u8>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.pipe);
            let _ = CloseHandle(self.event);
        }
    }
}

pub struct Client {
    name: Option<String>,
    connection: Option<Connection>,
    last_spawn: Option<u64>,
}

fn now_ms() -> u64 {
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut c_void), len, &mut len);
        let _ = CloseHandle(token);
        ok.ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut c_void)));
        sid
    }
}

/// 服务进程的位置（注册时写的）。
fn server_path() -> Option<String> {
    unsafe {
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let status = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(r"SOFTWARE\FreeType"),
            &HSTRING::from("ServerPath"),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        );
        if status.is_err() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
    }
}

/// 一次请求为什么没成。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fail {
    /// 服务进程没在规定时间里应答（忙、卡住）。
    Timeout,
    /// 连接断了（服务进程退出、重启）。
    Broken,
}

/// 等一个重叠操作完成；超时就取消。返回传输的字节数。
fn finish(conn: &Connection, overlapped: &mut OVERLAPPED, started: windows::core::Result<()>) -> Result<u32, Fail> {
    unsafe {
        if let Err(e) = started {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(Fail::Broken);
            }
            if WaitForSingleObject(conn.event, TIMEOUT_MS) != WAIT_OBJECT_0 {
                let _ = CancelIoEx(conn.pipe, Some(overlapped));
                let mut n = 0;
                let _ = GetOverlappedResult(conn.pipe, overlapped, &mut n, true);
                return Err(Fail::Timeout);
            }
        }
        let mut n = 0u32;
        GetOverlappedResult(conn.pipe, overlapped, &mut n, false).map_err(|_| Fail::Broken)?;
        Ok(n)
    }
}

impl Connection {
    fn open(name: &str) -> Result<Self, windows::core::Error> {
        let name = HSTRING::from(name);
        unsafe {
            let mut attempt = CreateFileW(
                &name,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            );
            if attempt.is_err() && GetLastError() == ERROR_PIPE_BUSY {
                let _ = WaitNamedPipeW(&name, 200);
                attempt = CreateFileW(
                    &name,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    None,
                );
            }
            let pipe = attempt?;
            let event = match CreateEventW(None, true, false, None) {
                Ok(event) => event,
                Err(e) => {
                    let _ = CloseHandle(pipe);
                    return Err(e);
                }
            };
            Ok(Self {
                pipe,
                event,
                buf: Vec::new(),
            })
        }
    }

    fn write(&self, mut data: &[u8]) -> Result<(), Fail> {
        while !data.is_empty() {
            let mut overlapped = OVERLAPPED {
                hEvent: self.event,
                ..Default::default()
            };
            let started = unsafe { WriteFile(self.pipe, Some(data), None, Some(&mut overlapped)) };
            let n = finish(self, &mut overlapped, started)?;
            if n == 0 {
                return Err(Fail::Broken);
            }
            data = &data[n as usize..];
        }
        Ok(())
    }

    fn read_line(&mut self) -> Result<Vec<u8>, Fail> {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let line = self.buf[..pos].to_vec();
                self.buf.drain(..=pos);
                return Ok(line);
            }
            let mut chunk = [0u8; 8192];
            let mut overlapped = OVERLAPPED {
                hEvent: self.event,
                ..Default::default()
            };
            let started = unsafe { ReadFile(self.pipe, Some(&mut chunk), None, Some(&mut overlapped)) };
            let n = finish(self, &mut overlapped, started)?;
            if n == 0 {
                return Err(Fail::Broken);
            }
            self.buf.extend_from_slice(&chunk[..n as usize]);
        }
    }

    fn call(&mut self, request: &Request) -> Result<Response, Fail> {
        self.write(&encode(request))?;
        let line = self.read_line()?;
        decode(&line).map_err(|_| Fail::Broken)
    }
}

impl Client {
    pub fn new() -> Self {
        Self {
            name: current_user_sid().map(|sid| pipe_name(&sid)),
            connection: None,
            last_spawn: None,
        }
    }

    fn spawn_server(&mut self) {
        let now = now_ms();
        if self.last_spawn.is_some_and(|at| now.saturating_sub(at) < SPAWN_INTERVAL_MS) {
            return;
        }
        self.last_spawn = Some(now);
        let Some(path) = server_path() else {
            return;
        };
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let mut command = std::process::Command::new(&path);
        if command
            .creation_flags(DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .is_err()
        {
            let _ = std::process::Command::new(&path)
                .creation_flags(DETACHED_PROCESS)
                .spawn();
        }
    }

    fn ensure(&mut self) -> bool {
        if self.connection.is_some() {
            return true;
        }
        let Some(name) = self.name.clone() else {
            crate::log::write("读不到当前用户", "管道名无法确定");
            return false;
        };
        match Connection::open(&name) {
            Ok(mut conn) => {
                let hello = conn.call(&Request::Hello {
                    session: true,
                    app: crate::log::app_name(),
                });
                match hello {
                    Ok(r) if r.ok => {
                        self.connection = Some(conn);
                        true
                    }
                    other => {
                        crate::log::write("连接服务进程失败", &format!("hello 没有成功：{other:?}"));
                        false
                    }
                }
            }
            Err(e) => {
                crate::log::write("连不上服务进程", &format!("{e}；尝试启动服务进程"));
                self.spawn_server();
                false
            }
        }
    }

    /// 发一条请求。连不上、超时、出错都返回 None（调用方把键交还给应用）。
    pub fn call(&mut self, request: &Request) -> Option<Response> {
        for attempt in 0..2 {
            if !self.ensure() {
                return None;
            }
            let result = self.connection.as_mut()?.call(request);
            match result {
                Ok(response) => {
                    if !response.ok {
                        crate::log::write("服务进程报错", response.error.as_deref().unwrap_or(""));
                        return None;
                    }
                    return Some(response);
                }
                Err(fail) => {
                    // 连接坏了：丢掉（服务进程那边的会话也随之丢掉）
                    self.connection = None;
                    if fail == Fail::Timeout {
                        crate::log::write("服务进程没有应答", &format!("{TIMEOUT_MS} ms 内没有应答，这个键交给应用"));
                        return None;
                    }
                    if attempt == 0 {
                        // 服务进程可能刚重启过：重连再发一次
                        continue;
                    }
                    crate::log::write("连接断开", "重连后仍然失败，这个键交给应用");
                }
            }
        }
        None
    }

    pub fn disconnect(&mut self) {
        self.connection = None;
    }
}
