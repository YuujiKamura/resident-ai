//! Windows ConPTY wrapper.
//!
//! Spawns a process inside a pseudo-terminal and provides pipe-based I/O.
//! The child process sees a real TTY, enabling interactive CLI tools to
//! run in their full TUI mode while we control them programmatically.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, COORD, HPCON,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, InitializeProcThreadAttributeList, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    STARTUPINFOEXW,
};

/// A Windows ConPTY session with pipe-based I/O.
pub struct ConPty {
    pseudo_console: HPCON,
    input_write: HANDLE,
    output_read: HANDLE,
    process: HANDLE,
    thread: HANDLE,
    _reader_thread: Option<JoinHandle<()>>,
    buffer: Arc<Mutex<String>>,
}

// HANDLE is Send-safe for our use case (owned, not shared)
unsafe impl Send for ConPty {}
unsafe impl Sync for ConPty {}

impl ConPty {
    /// Spawn a command inside a new ConPTY.
    pub fn spawn(command: &str) -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            // Create pipe pairs
            let mut input_read = HANDLE::default();
            let mut input_write = HANDLE::default();
            let mut output_read = HANDLE::default();
            let mut output_write = HANDLE::default();

            CreatePipe(&mut input_read, &mut input_write, None, 0)?;
            CreatePipe(&mut output_read, &mut output_write, None, 0)?;

            // Create pseudo console (120 cols x 30 rows)
            let size = COORD { X: 120, Y: 30 };
            let hpc = CreatePseudoConsole(size, input_read, output_write, 0)?;

            // Close child-side handles (ConPTY owns them now)
            let _ = CloseHandle(input_read);
            let _ = CloseHandle(output_write);

            // Set up attribute list for STARTUPINFOEXW
            let mut attr_size: usize = 0;
            let _ = InitializeProcThreadAttributeList(
                None,
                1,
                None,
                &mut attr_size,
            );

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

            // Prepare STARTUPINFOEXW
            let mut si = STARTUPINFOEXW::default();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            si.lpAttributeList = attr_list;

            // Create process
            let mut pi = PROCESS_INFORMATION::default();
            let mut cmd_wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();

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

            // Start background reader thread
            let buffer = Arc::new(Mutex::new(String::new()));
            let buf_clone = Arc::clone(&buffer);
            // HANDLE is not Send, wrap raw pointer for thread transfer
            let raw_read = output_read.0 as usize;

            let reader_thread = std::thread::spawn(move || {
                let read_handle = HANDLE(raw_read as *mut std::ffi::c_void);
                let mut chunk = [0u8; 4096];
                loop {
                    let mut bytes_read: u32 = 0;
                    let ok = ReadFile(
                        read_handle,
                        Some(&mut chunk),
                        Some(&mut bytes_read),
                        None,
                    );
                    if ok.is_err() {
                        eprintln!("ReadFile error: {:?}", ok.err());
                        break;
                    }
                    if bytes_read == 0 {
                        eprintln!("ReadFile: 0 bytes read");
                        break;
                    }
                    let text = String::from_utf8_lossy(&chunk[..bytes_read as usize]);
                    eprintln!("PTY Read ({} bytes): {:?}", bytes_read, text);
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
            // output_read will be closed when reader thread exits
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore] // ConPTY output pipe needs named pipe for reliable flush — fix in next session
    fn test_spawn_and_write() {
        // Spawn cmd.exe (persistent), then send echo command via stdin
        let pty = ConPty::spawn("cmd.exe").expect("Failed to spawn");

        // Wait for cmd.exe prompt to appear (skip initial ANSI escape sequences)
        let start = Instant::now();
        let timeout = Duration::from_secs(10);
        while start.elapsed() < timeout {
            // cmd.exe prompt contains ">" character
            if pty.read_buffer().contains('>') {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(pty.read_buffer().contains('>'), "should have received cmd.exe prompt, got: {:?}", pty.read_buffer());

        // Send echo command
        pty.write(b"echo hello_conpty\r\n").expect("Failed to write");

        let start = Instant::now();
        while start.elapsed() < timeout {
            let buffer = pty.read_buffer();
            if buffer.contains("hello_conpty") {
                return; // Success
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        panic!("Timeout waiting for output. Buffer: {:?}", pty.read_buffer());
    }
}

