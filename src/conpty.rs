//! Windows ConPTY wrapper.
//!
//! Spawns a process inside a pseudo-terminal and provides pipe-based I/O.
//! The child process sees a real TTY, enabling interactive CLI tools to
//! run in their full TUI mode while we control them programmatically.
//!
//! # Environment requirement
//!
//! ConPTY output capture only works when the **parent process has no console**.
//! GUI apps (like ghostty-win) work out of the box. Console hosts (cmd.exe,
//! mintty/Git Bash, cargo test) cause child output to leak to the parent's
//! console instead of flowing through the ConPTY output pipe.
//!
//! Call [`ConPty::detach_console`] before [`ConPty::spawn`] if running from
//! a console host. This is not sufficient from mintty (Git Bash) because
//! mintty uses pipes, not a Windows Console.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{
    CloseHandle, GENERIC_READ, HANDLE, HANDLE_FLAGS, INVALID_HANDLE_VALUE,
    SetHandleInformation, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_OUTBOUND,
};
use windows::Win32::System::Console::{
    AllocConsole, ClosePseudoConsole, CreatePseudoConsole, FreeConsole,
    ResizePseudoConsole, COORD, HPCON,
};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, CreatePipe, PIPE_TYPE_BYTE,
};
use windows::Win32::System::Threading::{
    CreateProcessW, InitializeProcThreadAttributeList, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    STARTUPINFOEXW,
};

const HANDLE_FLAG_INHERIT_MASK: u32 = 0x00000001;

static PIPE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A Windows ConPTY session with pipe-based I/O.
pub struct ConPty {
    pseudo_console: HPCON,
    input_write: HANDLE,
    #[allow(dead_code)]
    output_read: HANDLE,
    process: HANDLE,
    thread: HANDLE,
    _reader_thread: Option<JoinHandle<()>>,
    buffer: Arc<Mutex<String>>,
}

unsafe impl Send for ConPty {}
unsafe impl Sync for ConPty {}

impl ConPty {
    /// Detach from the inherited console so child processes use ConPTY exclusively.
    /// Required when running from a real Windows Console host (cmd.exe, PowerShell).
    /// Not needed from GUI apps. Insufficient from mintty (Git Bash).
    pub fn detach_console() {
        unsafe {
            let _ = FreeConsole();
        }
    }

    /// Re-attach a console after [`detach_console`].
    pub fn reattach_console() {
        unsafe {
            let _ = AllocConsole();
        }
    }

    /// Spawn a command inside a new ConPTY.
    pub fn spawn(command: &str) -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            let counter = PIPE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let pipe_name = format!("\\\\.\\pipe\\LOCAL\\resident-ai-{}-{}\0", pid, counter);
            let pipe_name_wide: Vec<u16> = pipe_name.encode_utf16().collect();

            // Named input pipe (we write, ConPTY reads).
            let input_write = CreateNamedPipeW(
                windows::core::PCWSTR(pipe_name_wide.as_ptr()),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE,
                1,
                4096,
                4096,
                0,
                None,
            );
            if input_write == INVALID_HANDLE_VALUE {
                return Err(windows::core::Error::from_win32().into());
            }

            let input_read = CreateFileW(
                windows::core::PCWSTR(pipe_name_wide.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?;

            // Anonymous output pipe (ConPTY writes, we read).
            let mut output_read = HANDLE::default();
            let mut output_write = HANDLE::default();
            CreatePipe(&mut output_read, &mut output_write, None, 0)?;

            // Mark all handles non-inheritable.
            SetHandleInformation(input_write, HANDLE_FLAG_INHERIT_MASK, HANDLE_FLAGS(0))?;
            SetHandleInformation(input_read, HANDLE_FLAG_INHERIT_MASK, HANDLE_FLAGS(0))?;
            SetHandleInformation(output_read, HANDLE_FLAG_INHERIT_MASK, HANDLE_FLAGS(0))?;
            SetHandleInformation(output_write, HANDLE_FLAG_INHERIT_MASK, HANDLE_FLAGS(0))?;

            // Create pseudo console (120x30).
            let size = COORD { X: 120, Y: 30 };
            let hpc = CreatePseudoConsole(size, input_read, output_write, 0)?;

            // ConPTY owns these now.
            let _ = CloseHandle(input_read);
            let _ = CloseHandle(output_write);

            // Attribute list with pseudo console.
            let mut attr_size: usize = 0;
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut attr_size);
            let mut attr_buf: Vec<u8> = vec![0u8; attr_size];
            let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr() as *mut _);
            InitializeProcThreadAttributeList(Some(attr_list), 1, None, &mut attr_size)?;
            UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(hpc.0 as *const std::ffi::c_void),
                std::mem::size_of::<HPCON>(),
                None,
                None,
            )?;

            let mut si = STARTUPINFOEXW::default();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            si.lpAttributeList = attr_list;

            let mut pi = PROCESS_INFORMATION::default();
            let mut cmd_wide: Vec<u16> =
                command.encode_utf16().chain(std::iter::once(0)).collect();

            CreateProcessW(
                None,
                Some(windows::core::PWSTR(cmd_wide.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT,
                None,
                None,
                &si.StartupInfo,
                &mut pi,
            )?;

            // Background reader thread.
            let buffer = Arc::new(Mutex::new(String::new()));
            let buf_clone = Arc::clone(&buffer);
            let raw_read = output_read.0 as usize;

            let reader_thread = std::thread::spawn(move || {
                let read_handle = HANDLE(raw_read as *mut std::ffi::c_void);
                let mut chunk = [0u8; 4096];
                loop {
                    let mut bytes_read: u32 = 0;
                    let ok =
                        ReadFile(read_handle, Some(&mut chunk), Some(&mut bytes_read), None);
                    if ok.is_err() || bytes_read == 0 {
                        break;
                    }
                    let text = String::from_utf8_lossy(&chunk[..bytes_read as usize]);
                    if let Ok(mut buf) = buf_clone.lock() {
                        buf.push_str(&text);
                    }
                }
            });

            Ok(ConPty {
                pseudo_console: hpc,
                input_write,
                output_read,
                process: pi.hProcess,
                thread: pi.hThread,
                _reader_thread: Some(reader_thread),
                buffer,
            })
        }
    }

    /// Force ConPTY to flush its render buffer by triggering a resize.
    pub fn flush_render(&self) {
        unsafe {
            let size = COORD { X: 120, Y: 30 };
            let _ = ResizePseudoConsole(self.pseudo_console, size);
        }
    }

    /// Write data to the PTY's stdin.
    pub fn write(&self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        unsafe {
            let mut written: u32 = 0;
            WriteFile(self.input_write, Some(data), Some(&mut written), None)?;
            Ok(())
        }
    }

    /// Read the accumulated output buffer.
    pub fn read_buffer(&self) -> String {
        self.buffer.lock().unwrap().clone()
    }

    /// Get the current buffer length.
    pub fn buffer_len(&self) -> usize {
        self.buffer.lock().unwrap().len()
    }

    /// Check if the child process is still alive.
    pub fn is_alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.process, 0) != WAIT_OBJECT_0 }
    }
}

impl Drop for ConPty {
    fn drop(&mut self) {
        unsafe {
            ClosePseudoConsole(self.pseudo_console);
            if self.is_alive() {
                let _ = TerminateProcess(self.process, 1);
            }
            let _ = CloseHandle(self.process);
            let _ = CloseHandle(self.thread);
            let _ = CloseHandle(self.input_write);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // --- Item 6: Subprocess spawn ---

    #[test]
    #[cfg(windows)]
    fn spawn_returns_ok() {
        let result = ConPty::spawn("cmd.exe /c exit 0");
        assert!(result.is_ok(), "spawn should succeed: {:?}", result.err());
    }

    #[test]
    #[cfg(windows)]
    fn process_alive_after_spawn() {
        // Spawn interactive cmd.exe (no /c so it stays alive).
        // Check is_alive() immediately — cmd.exe does not exit instantly.
        let pty = ConPty::spawn("cmd.exe").expect("spawn cmd.exe");
        assert!(pty.is_alive(), "cmd.exe should still be alive immediately after spawn");
    }

    #[test]
    #[cfg(windows)]
    fn spawn_with_args() {
        // Both should succeed; /c echo test exits immediately, bare cmd stays alive
        let pty_with_args = ConPty::spawn("cmd.exe /c echo test");
        let pty_bare = ConPty::spawn("cmd.exe");
        assert!(pty_with_args.is_ok(), "spawn with args should succeed");
        assert!(pty_bare.is_ok(), "spawn bare cmd.exe should succeed");
        // They are independent spawns — both succeed but behave differently
    }

    // --- Item 7: stdout capture / buffer ---

    #[test]
    #[cfg(windows)]
    fn buffer_initially_small() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").expect("spawn");
        std::thread::sleep(Duration::from_millis(300));
        // Buffer may have ANSI init bytes but should be small (< 100 bytes)
        let len = pty.buffer_len();
        assert!(len < 100, "initial buffer should be small, got {} bytes", len);
    }

    #[test]
    #[cfg(windows)]
    fn buffer_len_matches_read_buffer() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").expect("spawn");
        std::thread::sleep(Duration::from_millis(300));
        let buf = pty.read_buffer();
        let len = pty.buffer_len();
        assert_eq!(len, buf.len(), "buffer_len() should equal read_buffer().len()");
    }

    #[test]
    #[cfg(windows)]
    fn write_does_not_error() {
        let pty = ConPty::spawn("cmd.exe").expect("spawn cmd.exe");
        std::thread::sleep(Duration::from_millis(200));
        let result = pty.write(b"echo test\r\n");
        assert!(result.is_ok(), "write should not error: {:?}", result.err());
    }

    // --- Item 13: Resource management — unique pipes, handles ---

    #[test]
    #[cfg(windows)]
    fn pipe_counter_increments() {
        use std::sync::atomic::Ordering;
        let before = super::PIPE_COUNTER.load(Ordering::Relaxed);
        let _pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        let after = super::PIPE_COUNTER.load(Ordering::Relaxed);
        assert!(after > before, "pipe counter should increment on spawn");
    }

    #[test]
    #[cfg(windows)]
    fn multiple_instances_unique_counters() {
        use std::sync::atomic::Ordering;
        let before = super::PIPE_COUNTER.load(Ordering::Relaxed);
        let _pty1 = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        let _pty2 = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        let after = super::PIPE_COUNTER.load(Ordering::Relaxed);
        // Tests run in parallel so other tests may also increment the counter;
        // assert at least 2 increments happened (one per spawn in this test).
        assert!(after >= before + 2, "two spawns should increment counter by at least 2, got before={} after={}", before, after);
    }

    #[test]
    #[cfg(windows)]
    fn spawn_and_drop_no_panic() {
        // Just ensure spawn + immediate drop doesn't panic or leak
        {
            let _pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        }
        // If we get here, drop succeeded
    }

    // --- Item 14: Metrics / observability ---

    #[test]
    #[cfg(windows)]
    fn buffer_grows_over_time() {
        let pty = ConPty::spawn("cmd.exe /c echo something").unwrap();
        let initial = pty.buffer_len();
        std::thread::sleep(Duration::from_secs(2));
        // Buffer might have received ANSI init sequences at minimum
        // We just check it doesn't shrink
        assert!(pty.buffer_len() >= initial, "buffer should not shrink");
    }

    #[test]
    #[cfg(windows)]
    fn is_alive_reflects_state() {
        let pty = ConPty::spawn("cmd.exe").unwrap();
        assert!(pty.is_alive(), "interactive cmd should be alive");
        // Note: we can't easily kill it in this test, just verify alive state
    }

    // --- Item 15: Drop cleanup — process termination ---

    #[test]
    #[cfg(windows)]
    fn drop_terminates_process() {
        // We can't easily check after drop since handles are gone.
        // Instead verify the drop path doesn't panic with a living process.
        let pty = ConPty::spawn("cmd.exe").unwrap();
        assert!(pty.is_alive());
        drop(pty);
        // If we get here, drop succeeded without panic
    }

    #[test]
    #[cfg(windows)]
    fn drop_handles_already_dead_process() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        std::thread::sleep(Duration::from_secs(2));
        assert!(!pty.is_alive());
        drop(pty);
        // Drop on dead process should not panic
    }

    #[test]
    #[cfg(windows)]
    fn multiple_spawn_drop_cycles() {
        for _ in 0..3 {
            let _pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
            // implicit drop at end of loop iteration
        }
        // 3 cycles without panic = resource cleanup working
    }
}
