//! 命名管道：每个连接一个线程，读一行请求、交给引擎线程、写回一行应答。
//!
//! 管道的访问控制：当前用户、SYSTEM、应用容器（商店应用、开始菜单搜索框）可读写，
//! 完整性标签设为低，低完整性进程（浏览器沙箱等）里的输入法也能连上。

use std::ffi::c_void;
use std::io;
use std::sync::mpsc::{self, Sender};

use freetype_protocol::{decode, encode, Request, Response};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, GENERIC_READ,
    GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};

use crate::host::ClientId;

/// 一行请求最长多少字节（快捷短语整份替换也够用）。
const MAX_LINE: usize = 16 << 20;

/// 交给引擎线程的消息。
pub enum Message {
    Connected(ClientId),
    Request(ClientId, Request, Sender<Response>),
    Disconnected(ClientId),
}

/// 当前用户的 SID（S-1-5-21-...）。
pub fn current_user_sid() -> io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let result = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            len,
            &mut len,
        );
        let _ = CloseHandle(token);
        result?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let sid = text.to_string().map_err(io::Error::other);
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut c_void)));
        sid
    }
}

/// 同一登录会话里只跑一个服务进程。返回 false 表示已经有一个了。
pub fn single_instance() -> bool {
    unsafe {
        let name = format!(r"Local\FreeType.Server{}", crate::instance_suffix());
        match CreateMutexW(None, true, &HSTRING::from(name)) {
            // 句柄故意不关：进程退出时系统收回
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => false,
        }
    }
}

struct Pipe(HANDLE);

// SAFETY: 管道句柄只在拿着它的那个线程里读写。
unsafe impl Send for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

fn read_some(handle: HANDLE, buf: &mut [u8]) -> io::Result<usize> {
    let mut read = 0u32;
    unsafe { ReadFile(handle, Some(buf), Some(&mut read), None)? };
    Ok(read as usize)
}

fn write_all(handle: HANDLE, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let mut written = 0u32;
        unsafe { WriteFile(handle, Some(data), Some(&mut written), None)? };
        data = &data[written as usize..];
    }
    Ok(())
}

/// 按行读：返回一行（不含 `\n`），连接断了返回 None。
struct LineReader {
    buf: Vec<u8>,
}

impl LineReader {
    fn next_line(&mut self, handle: HANDLE) -> Option<Vec<u8>> {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let line = self.buf[..pos].to_vec();
                self.buf.drain(..=pos);
                return Some(line);
            }
            if self.buf.len() > MAX_LINE {
                return None;
            }
            let mut chunk = [0u8; 4096];
            match read_some(handle, &mut chunk) {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
            }
        }
    }
}

fn serve(pipe: Pipe, id: ClientId, engine: Sender<Message>) {
    let _ = engine.send(Message::Connected(id));
    let mut reader = LineReader { buf: Vec::new() };
    while let Some(line) = reader.next_line(pipe.0) {
        let response = match decode::<Request>(&line) {
            Ok(request) => {
                let (tx, rx) = mpsc::channel();
                if engine.send(Message::Request(id, request, tx)).is_err() {
                    break;
                }
                match rx.recv() {
                    Ok(response) => response,
                    Err(_) => break,
                }
            }
            Err(e) => Response::error(format!("请求格式不对：{e}")),
        };
        if write_all(pipe.0, &encode(&response)).is_err() {
            break;
        }
        unsafe {
            let _ = FlushFileBuffers(pipe.0);
        }
    }
    let _ = engine.send(Message::Disconnected(id));
}

fn security_descriptor(sid: &str) -> io::Result<PSECURITY_DESCRIPTOR> {
    // 当前用户、SYSTEM 完全控制；应用容器（AC 和受限 AC）读写；低完整性也能写
    let sddl = format!(
        "D:P(A;;GA;;;{sid})(A;;GA;;;SY)(A;;GRGW;;;AC)(A;;GRGW;;;S-1-15-2-2)S:(ML;;NW;;;LW)"
    );
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )?;
    }
    Ok(sd)
}

/// 一直接受连接（不返回，除非建不了管道）。
pub fn listen(name: &str, sid: &str, engine: Sender<Message>) -> io::Result<()> {
    let sd = security_descriptor(sid)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let name = HSTRING::from(name);
    let mut next_id: ClientId = 1;
    loop {
        let handle = unsafe {
            CreateNamedPipeW(
                &name,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                Some(&attributes),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let pipe = Pipe(handle);
        let connected = unsafe { ConnectNamedPipe(pipe.0, None) };
        if let Err(e) = connected {
            if e.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                continue;
            }
        }
        let id = next_id;
        next_id += 1;
        let engine = engine.clone();
        std::thread::Builder::new()
            .name(format!("pipe-{id}"))
            .spawn(move || serve(pipe, id, engine))?;
    }
}

/// 简单的客户端：连上、发一条、读一条（`--stop`、测试用）。
pub fn request(name: &str, request: &Request) -> io::Result<Response> {
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(name),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )?
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let handle = Handle(handle);
    write_all(handle.0, &encode(request))?;
    let mut reader = LineReader { buf: Vec::new() };
    let line = reader
        .next_line(handle.0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "服务进程没有应答"))?;
    decode(&line).map_err(io::Error::other)
}
