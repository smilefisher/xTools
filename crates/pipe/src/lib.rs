//! 本地命名管道：一行一个 JSON，请求/响应式。
//!
//! 为什么不用 TCP：本地管道不需要端口、不受防火墙影响，权限也能限定到当前用户。

#[cfg(windows)]
mod imp {
    use std::io;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    /// 管道全路径（UTF-16，带结尾 0）。
    fn pipe_path(name: &str) -> Vec<u16> {
        format!("\\\\.\\pipe\\{name}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    /// 客户端：每次请求新建连接、用完即关。
    pub struct Client {
        handle: HANDLE,
    }

    impl Client {
        pub fn connect(name: &str, timeout_ms: u32) -> io::Result<Self> {
            let path = pipe_path(name);
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);

            loop {
                let handle = unsafe {
                    CreateFileW(
                        PCWSTR(path.as_ptr()),
                        FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        None,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        None,
                    )
                };

                match handle {
                    Ok(handle) => return Ok(Self { handle }),
                    Err(err) => {
                        if std::time::Instant::now() >= deadline {
                            return Err(io::Error::from_raw_os_error(err.code().0));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
            }
        }

        /// 发送一行请求，读取一行响应。
        pub fn request(&self, payload: &str) -> io::Result<String> {
            let mut data = payload.as_bytes().to_vec();
            data.push(b'\n');

            let mut written = 0u32;
            unsafe {
                WriteFile(self.handle, Some(&data), Some(&mut written), None)
                    .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
            }

            let mut collected = Vec::with_capacity(4096);
            let mut buffer = [0u8; 4096];
            loop {
                let mut read = 0u32;
                unsafe {
                    ReadFile(self.handle, Some(&mut buffer), Some(&mut read), None)
                        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
                }
                if read == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "服务端关闭连接"));
                }
                collected.extend_from_slice(&buffer[..read as usize]);
                if collected.last() == Some(&b'\n') {
                    collected.pop();
                    return String::from_utf8(collected)
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "响应不是 UTF-8"));
                }
            }
        }
    }

    impl Drop for Client {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }

    /// 服务端：循环接连接，每个连接一个线程，读一行处理一行。
    pub fn serve<F>(name: &str, handler: F) -> io::Result<()>
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let path = pipe_path(name);
        let handler = std::sync::Arc::new(handler);

        loop {
            let handle = unsafe {
                CreateNamedPipeW(
                    PCWSTR(path.as_ptr()),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    1 << 20,
                    1 << 20,
                    0,
                    None,
                )
            };

            if handle.is_invalid() {
                return Err(io::Error::last_os_error());
            }

            if unsafe { ConnectNamedPipe(handle, None) }.is_err() {
                // 客户端在两次连接之间溜走属于正常情况
                unsafe {
                    let _ = CloseHandle(handle);
                }
                continue;
            }

            let handler = std::sync::Arc::clone(&handler);
            // HANDLE 不是 Send，转成裸指针再在子线程里还原
            let raw = handle.0 as isize;
            std::thread::spawn(move || {
                let handle = HANDLE(raw as *mut std::ffi::c_void);
                serve_connection(handle, handler);
                unsafe {
                    let _ = DisconnectNamedPipe(handle);
                    let _ = CloseHandle(handle);
                }
            });
        }
    }

    fn serve_connection(
        handle: HANDLE,
        handler: std::sync::Arc<impl Fn(&str) -> String + Send + Sync>,
    ) {
        let mut buffer = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();

        loop {
            let mut read = 0u32;
            let ok = unsafe { ReadFile(handle, Some(&mut buffer), Some(&mut read), None) };
            if ok.is_err() || read == 0 {
                return;
            }
            pending.extend_from_slice(&buffer[..read as usize]);

            while let Some(index) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=index).collect();
                let request = String::from_utf8_lossy(&line[..line.len() - 1]).to_string();
                let mut response = handler(&request).into_bytes();
                response.push(b'\n');

                let mut written = 0u32;
                if unsafe { WriteFile(handle, Some(&response), Some(&mut written), None) }.is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{serve, Client};

/// 非 Windows 平台给个明确报错，避免 cfg 到处飞。
#[cfg(not(windows))]
pub fn serve<F>(_name: &str, _handler: F) -> std::io::Result<()>
where
    F: Fn(&str) -> String + Send + Sync + 'static,
{
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "仅支持 Windows",
    ))
}
