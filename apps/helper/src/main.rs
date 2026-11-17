//! Elevated helper (D-03). No UI. A small, fixed protocol over a verified pipe.
//! It has no general command execution and never reads file contents.
#![windows_subsystem = "windows"]

use oz_ops::{execute, Guard, Plan};
use oz_win::elevate::{connect_to_parent, is_process_alive, read_frame, tag, write_frame, Pipe};
use std::sync::atomic::{AtomicBool, Ordering};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return;
    }
    let (Ok(parent), Ok(token)) = (args[2].parse::<u32>(), u128::from_str_radix(&args[3], 16)) else { return };
    let Ok(mut pipe) = connect_to_parent(&args[1], parent, token) else { return };

    // Exit with the parent, whatever it is doing.
    std::thread::spawn(move || loop {
        if !is_process_alive(parent) {
            std::process::exit(0);
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    });

    // One request at a time; a closed pipe ends the helper.
    while let Ok((t, data)) = read_frame(&mut pipe) {
        let ok = match t {
            tag::SCAN => scan(&mut pipe, &data),
            tag::PLAN => plan(&mut pipe, &data),
            tag::COMPONENT_CLEANUP => cleanup(&mut pipe),
            tag::SHUTDOWN => return,
            _ => write_frame(&mut pipe, tag::ERROR, b"unknown request").is_ok(),
        };
        if !ok {
            return;
        }
    }
}

fn scan(pipe: &mut Pipe, data: &[u8]) -> bool {
    let Some(&letter) = data.first().filter(|c| c.is_ascii_alphabetic()) else {
        return write_frame(pipe, tag::ERROR, b"bad volume").is_ok();
    };
    let cancel = AtomicBool::new(false);
    let mut buf = Vec::with_capacity(1 << 20);
    let result = oz_win::mft::read_records(letter as char, &cancel, |r| {
        oz_ntfs::encode_record(&r, &mut buf);
        if buf.len() >= (1 << 20) {
            // A closed pipe means the core cancelled or went away.
            if write_frame(pipe, tag::RECORDS, &buf).is_err() {
                cancel.store(true, Ordering::Relaxed);
            }
            buf.clear();
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return false;
    }
    if !buf.is_empty() && write_frame(pipe, tag::RECORDS, &buf).is_err() {
        return false;
    }
    match result {
        Ok(n) => write_frame(pipe, tag::SCAN_DONE, &n.to_le_bytes()).is_ok(),
        Err(e) => write_frame(pipe, tag::ERROR, e.as_bytes()).is_ok(),
    }
}

fn plan(pipe: &mut Pipe, data: &[u8]) -> bool {
    let Ok(plan) = serde_json::from_slice::<Plan>(data) else {
        return write_frame(pipe, tag::ERROR, b"bad plan").is_ok();
    };
    // Independent policy check: the helper never trusts the UI process.
    let guard = match Guard::from_env() {
        Ok(g) => g,
        Err(e) => return write_frame(pipe, tag::ERROR, e.0.as_bytes()).is_ok(),
    };
    let report = execute(&plan, &guard, None, &AtomicBool::new(false), |_| {});
    let body = serde_json::to_vec(&report).unwrap_or_default();
    write_frame(pipe, tag::REPORT, &body).is_ok()
}

/// Microsoft's supported cleanup only, never /ResetBase.
fn cleanup(pipe: &mut Pipe) -> bool {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let status = std::process::Command::new(format!(r"{windir}\System32\Dism.exe"))
        .args(["/Online", "/Cleanup-Image", "/StartComponentCleanup", "/Quiet", "/NoRestart"])
        .status();
    let msg = match status {
        Ok(s) => s.code().unwrap_or(-1).to_string(),
        Err(e) => e.to_string(),
    };
    write_frame(pipe, tag::TOOL_DONE, msg.as_bytes()).is_ok()
}
