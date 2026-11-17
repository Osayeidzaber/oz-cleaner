//! Launches the elevated helper and connects it over a named pipe (D-03).
//! The unelevated core owns the pipe. Each side verifies the other's process ID.
use crate::wide;
use std::io::{Read, Write};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

pub struct Pipe(HANDLE);
// SAFETY: a pipe handle may be used from any thread; access here is sequential.
unsafe impl Send for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: owned handle.
        unsafe { CloseHandle(self.0) };
    }
}

impl Read for Pipe {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        // SAFETY: buffer valid for its length.
        let ok = unsafe {
            ReadFile(
                self.0,
                b.as_mut_ptr(),
                b.len().min(1 << 30) as u32,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            // SAFETY: no preconditions.
            let e = unsafe { GetLastError() };
            if e == ERROR_BROKEN_PIPE {
                return Ok(0);
            }
            return Err(std::io::Error::from_raw_os_error(e as i32));
        }
        Ok(n as usize)
    }
}

impl Write for Pipe {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        // SAFETY: buffer valid for its length.
        let ok = unsafe {
            WriteFile(
                self.0,
                b.as_ptr(),
                b.len().min(1 << 30) as u32,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub enum LaunchError {
    Declined,
    Failed(String),
}

fn random_token() -> u128 {
    use std::hash::{BuildHasher, Hasher};
    let a = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish() as u128;
    let b = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish() as u128;
    a << 64 | b
}

/// Shows UAC, starts the helper and returns a verified, connected pipe.
pub fn launch_helper(exe: &str) -> Result<Pipe, LaunchError> {
    let token = random_token();
    let name = format!(r"\\.\pipe\oz-cleaner-{token:032x}");
    let wname = wide(&name);
    // SAFETY: NUL-terminated name. Default DACL: this user, SYSTEM, admins.
    // FIRST_PIPE_INSTANCE refuses a pre-existing pipe of this name.
    let pipe = unsafe {
        CreateNamedPipeW(
            wname.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1 << 20,
            1 << 20,
            0,
            std::ptr::null(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(LaunchError::Failed("could not create pipe".into()));
    }
    let pipe = Pipe(pipe);

    // SAFETY: no preconditions.
    let me = unsafe { GetCurrentProcessId() };
    let params = wide(&format!("{name} {me} {token:032x}"));
    let (verb, file) = (wide("runas"), wide(exe));
    // SAFETY: zeroed POD then initialized; strings outlive the call.
    let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    sei.fMask = SEE_MASK_NOCLOSEPROCESS;
    sei.lpVerb = verb.as_ptr();
    sei.lpFile = file.as_ptr();
    sei.lpParameters = params.as_ptr();
    sei.nShow = SW_HIDE;
    // SAFETY: sei is valid.
    if unsafe { ShellExecuteExW(&mut sei) } == 0 {
        // SAFETY: no preconditions.
        let e = unsafe { GetLastError() };
        return Err(if e == ERROR_CANCELLED {
            LaunchError::Declined
        } else {
            LaunchError::Failed(format!("launch failed: {e}"))
        });
    }
    let child = sei.hProcess;
    // SAFETY: child handle from ShellExecuteEx.
    let child_pid = unsafe { GetProcessId(child) };

    // Unblock ConnectNamedPipe if the helper dies before connecting.
    let wname2 = name.clone();
    let child_raw = child as isize;
    std::thread::spawn(move || {
        // SAFETY: handle stays open for the life of this waiter, which closes it.
        unsafe {
            WaitForSingleObject(child_raw as HANDLE, INFINITE);
            CloseHandle(child_raw as HANDLE);
        }
        let w = wide(&wname2);
        // A dummy connection releases a blocked ConnectNamedPipe; PID check rejects it.
        // SAFETY: NUL-terminated name.
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            // SAFETY: just opened.
            unsafe { CloseHandle(h) };
        }
    });

    // SAFETY: open pipe; synchronous mode.
    let ok = unsafe { ConnectNamedPipe(pipe.0, std::ptr::null_mut()) };
    // SAFETY: no preconditions.
    if ok == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
        return Err(LaunchError::Failed("helper did not connect".into()));
    }
    let mut client = 0u32;
    // SAFETY: open pipe; valid out pointer.
    if unsafe { GetNamedPipeClientProcessId(pipe.0, &mut client) } == 0 || client != child_pid {
        return Err(LaunchError::Failed("pipe client was not the helper".into()));
    }
    let mut p = pipe;
    let mut got = [0u8; 16];
    p.read_exact(&mut got)
        .map_err(|e| LaunchError::Failed(format!("handshake: {e}")))?;
    if u128::from_le_bytes(got) != token {
        return Err(LaunchError::Failed("handshake token mismatch".into()));
    }
    Ok(p)
}

/// Helper side: connects, verifies the server's PID, and returns the pipe.
pub fn connect_to_parent(name: &str, parent_pid: u32, token: u128) -> Result<Pipe, String> {
    let w = wide(name);
    // SAFETY: NUL-terminated name.
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err("cannot open pipe".into());
    }
    let mut p = Pipe(h);
    let mut server = 0u32;
    // SAFETY: open pipe; valid out pointer.
    if unsafe { GetNamedPipeServerProcessId(p.0, &mut server) } == 0 || server != parent_pid {
        return Err("pipe server is not the parent".into());
    }
    p.write_all(&token.to_le_bytes())
        .map_err(|e| e.to_string())?;
    Ok(p)
}

pub fn is_process_alive(pid: u32) -> bool {
    // SAFETY: handle closed below.
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return false;
        }
        let r = WaitForSingleObject(h, 0);
        CloseHandle(h);
        r == WAIT_TIMEOUT
    }
}

/// Protocol tags. Closing the pipe is how the core cancels a running scan.
pub mod tag {
    pub const SCAN: u8 = 1;
    pub const PLAN: u8 = 3;
    pub const COMPONENT_CLEANUP: u8 = 4;
    pub const SHUTDOWN: u8 = 5;
    pub const TWEAK_APPLY: u8 = 6;
    pub const TWEAK_REVERT: u8 = 7;
    pub const TWEAK_DONE: u8 = 15;
    pub const RECORDS: u8 = 10;
    pub const SCAN_DONE: u8 = 11;
    pub const ERROR: u8 = 12;
    pub const REPORT: u8 = 13;
    pub const TOOL_DONE: u8 = 14;
}

// Length-prefixed frames shared by both ends: tag byte, u32 length, payload.
pub fn write_frame(w: &mut impl Write, tag: u8, data: &[u8]) -> std::io::Result<()> {
    w.write_all(&[tag])?;
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data)
}

pub fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 5];
    r.read_exact(&mut h)?;
    let n = u32::from_le_bytes([h[1], h[2], h[3], h[4]]) as usize;
    if n > 64 << 20 {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut b = vec![0u8; n];
    r.read_exact(&mut b)?;
    Ok((h[0], b))
}
