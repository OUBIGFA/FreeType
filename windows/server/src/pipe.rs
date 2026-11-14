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
    GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, ERROR_INSUFFICIENT_BUFFER,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, RevertToSelf, TokenIntegrityLevel,
    TokenIsAppContainer, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_INFORMATION_CLASS,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, ImpersonateNamedPipeClient, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::SystemServices::SECURITY_MANDATORY_MEDIUM_RID;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken};

use crate::host::ClientId;

/// 一行请求最长多少字节（快捷短语整份替换也够用）。
const MAX_LINE: usize = 16 << 20;

/// 交给引擎线程的消息。
pub enum Message {
    Connected(ClientId),
    Request(ClientId, Request, Sender<Response>),
    Disconnected(ClientId),
}

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.0); }
    }
}

fn token_information(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<usize>> {
    let mut length = 0;
    unsafe {
        if let Err(error) = GetTokenInformation(token, class, None, 0, &mut length) {
            if error.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() { return Err(error.into()); }
        }
        // TOKEN_USER 等结构含指针，缓冲区需满足指针对齐，而不仅是字节对齐。
        let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(token, class, Some(buffer.as_mut_ptr().cast()), length, &mut length)?;
        Ok(buffer)
    }
}

fn token_sid(token: HANDLE) -> io::Result<String> {
    let buffer = token_information(token, TokenUser)?;
    unsafe {
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let sid = text.to_string().map_err(io::Error::other);
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut c_void)));
        sid
    }
}

/// 当前用户的 SID（S-1-5-21-...）。
pub fn current_user_sid() -> io::Result<String> {
    let mut handle = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle)?; }
    let token = Token(handle);
    token_sid(token.0)
}

fn client_can_manage(pipe: HANDLE, owner: &str) -> io::Result<bool> {
    // 只用操作系统提供的客户端令牌，不相信 Hello 中的应用名称或 session 标志。
    unsafe { ImpersonateNamedPipeClient(pipe)?; }
    let mut handle = HANDLE::default();
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut handle) };
    let token = Token(handle);
    // 在解析请求或交给引擎之前恢复身份；失败时调用方必须结束此连接线程。
    unsafe { RevertToSelf()?; }
    opened?;
    let sid = token_sid(token.0)?;
    if sid != owner && sid != "S-1-5-18" {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "客户端不是当前用户"));
    }
    let integrity = token_information(token.0, TokenIntegrityLevel)?;
    let container = token_information(token.0, TokenIsAppContainer)?;
    unsafe {
        let label = &*integrity.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        let level = *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1);
        let app_container = *container.as_ptr().cast::<u32>() != 0;
        Ok(level >= SECURITY_MANDATORY_MEDIUM_RID as u32 && !app_container)
    }
}

fn input_request(request: &Request) -> bool {
    matches!(request,
        Request::Hello { .. } | Request::Key { .. } | Request::Select { .. } | Request::Page { .. }
        | Request::Forget { .. } | Request::Reset { .. } | Request::Focus { .. }
        | Request::ToggleMode { .. } | Request::Command { .. } | Request::Status)
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

fn serve(pipe: Pipe, id: ClientId, engine: Sender<Message>, owner: &str) {
    let _ = engine.send(Message::Connected(id));
    let mut reader = LineReader { buf: Vec::new() };
    while let Some(line) = reader.next_line(pipe.0) {
        let can_manage = match client_can_manage(pipe.0, owner) {
            Ok(allowed) => allowed,
            Err(error) => {
                crate::log(&format!("管道客户端身份检查失败：{error}"));
                break;
            }
        };
        let response = match decode::<Request>(&line) {
            Ok(request) if !can_manage && !input_request(&request) => {
                Response::error("受限应用只能发送输入请求，不能管理或导出用户数据")
            }
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
        let owner = sid.to_string();
        std::thread::Builder::new()
            .name(format!("pipe-{id}"))
            .spawn(move || serve(pipe, id, engine, &owner))?;
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
