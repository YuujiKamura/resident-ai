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

    /// ConPTY init sequence: enable win32-input-mode + enable focus reporting
    const CONPTY_INIT: &str = "\x1b[?9001h\x1b[?1004h";

    // === spawn lifecycle ===

    #[test]
    #[cfg(windows)]
    fn spawn_valid_command() {
        assert!(ConPty::spawn("cmd.exe /c exit 0").is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn spawn_interactive_is_alive() {
        let pty = ConPty::spawn("cmd.exe").unwrap();
        assert_eq!(pty.is_alive(), true);
    }

    #[test]
    #[cfg(windows)]
    fn spawn_exit_command_is_dead() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(pty.is_alive(), false);
    }

    // === pipe naming ===

    #[test]
    #[cfg(windows)]
    fn each_spawn_increments_counter_by_one() {
        use std::sync::atomic::Ordering;
        let before = PIPE_COUNTER.load(Ordering::Relaxed);
        let _pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        assert_eq!(PIPE_COUNTER.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    #[cfg(windows)]
    fn two_spawns_increment_counter_by_two() {
        use std::sync::atomic::Ordering;
        let before = PIPE_COUNTER.load(Ordering::Relaxed);
        let _pty1 = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        let _pty2 = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        assert_eq!(PIPE_COUNTER.load(Ordering::Relaxed), before + 2);
    }

    // === buffer behavior ===

    #[test]
    #[cfg(windows)]
    fn buffer_starts_with_conpty_init() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let buf = pty.read_buffer();
        assert!(
            buf.starts_with(CONPTY_INIT),
            "buffer should start with ConPTY init sequence, got {:?}",
            &buf[..buf.len().min(32)]
        );
    }

    #[test]
    #[cfg(windows)]
    fn buffer_init_is_16_bytes() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        // ConPTY emits exactly 16 bytes of init before any child output
        // (from mintty/console host, no further text content arrives)
        assert_eq!(pty.read_buffer().len(), CONPTY_INIT.len());
    }

    #[test]
    #[cfg(windows)]
    fn buffer_len_equals_read_buffer_len() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(pty.buffer_len(), pty.read_buffer().len());
    }

    // === write ===

    #[test]
    #[cfg(windows)]
    fn write_to_living_process_succeeds() {
        let pty = ConPty::spawn("cmd.exe").unwrap();
        assert_eq!(pty.write(b"echo test\r\n").is_ok(), true);
    }

    // === drop cleanup ===

    #[test]
    #[cfg(windows)]
    fn drop_living_process_no_panic() {
        let pty = ConPty::spawn("cmd.exe").unwrap();
        assert_eq!(pty.is_alive(), true);
        drop(pty);
    }

    #[test]
    #[cfg(windows)]
    fn drop_dead_process_no_panic() {
        let pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(pty.is_alive(), false);
        drop(pty);
    }

    #[test]
    #[cfg(windows)]
    fn three_spawn_drop_cycles() {
        for i in 0..3 {
            let pty = ConPty::spawn("cmd.exe /c exit 0").unwrap();
            std::thread::sleep(Duration::from_millis(500));
            assert_eq!(pty.is_alive(), false, "cycle {} should be dead", i);
            drop(pty);
        }
    }
}
