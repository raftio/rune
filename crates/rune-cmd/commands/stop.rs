use anyhow::{bail, Result};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::path::Path;

use crate::cli::StopArgs;

const GRACEFUL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

pub fn exec(args: StopArgs) -> Result<()> {
    let pid_path = Path::new(&args.pid_file);

    if !pid_path.exists() {
        bail!("pid file not found: {} (is rune running?)", args.pid_file);
    }

    let raw = std::fs::read_to_string(pid_path)?;
    let pid: i32 = raw.trim().parse().map_err(|_| {
        anyhow::anyhow!("invalid pid in {}: {:?}", args.pid_file, raw.trim())
    })?;

    if !process_alive(pid) {
        std::fs::remove_file(pid_path)?;
        println!();
        println!("  \x1b[33m▲\x1b[0m Process {pid} not running \x1b[2m(stale PID file removed)\x1b[0m");
        println!();
        return Ok(());
    }

    println!();
    println!("  \x1b[33m●\x1b[0m Stopping Rune \x1b[2m(pid {pid})\x1b[0m...");
    signal::kill(Pid::from_raw(pid), Signal::SIGTERM)?;

    let deadline = std::time::Instant::now() + GRACEFUL_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if !process_alive(pid) {
            let _ = std::fs::remove_file(pid_path);
            println!("  \x1b[32m▲\x1b[0m \x1b[1mRune stopped\x1b[0m");
            println!();
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    eprintln!("  \x1b[31m✕\x1b[0m Process did not exit gracefully, forcing...");
    signal::kill(Pid::from_raw(pid), Signal::SIGKILL)?;
    std::thread::sleep(POLL_INTERVAL);

    let _ = std::fs::remove_file(pid_path);
    println!("  \x1b[32m▲\x1b[0m \x1b[1mRune stopped\x1b[0m \x1b[2m(forced)\x1b[0m");
    println!();
    Ok(())
}

fn process_alive(pid: i32) -> bool {
    signal::kill(Pid::from_raw(pid), None).is_ok()
}
