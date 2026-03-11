use anyhow::{bail, Result};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::path::Path;

use crate::cli::DebugArgs;

const GRACEFUL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

pub async fn exec(args: DebugArgs) -> Result<()> {
    if args.restart {
        restart_daemon(&args)?;
        return Ok(());
    }

    tail_log(&args.log_file)
}

fn restart_daemon(args: &DebugArgs) -> Result<()> {
    let pid_path = Path::new(&args.pid_file);

    if pid_path.exists() {
        let raw = std::fs::read_to_string(pid_path)?;
        let pid: i32 = raw.trim().parse().map_err(|_| {
            anyhow::anyhow!("invalid pid in {}: {:?}", args.pid_file, raw.trim())
        })?;

        if process_alive(pid) {
            println!();
            println!("  \x1b[33m●\x1b[0m Stopping Rune \x1b[2m(pid {pid})\x1b[0m...");
            signal::kill(Pid::from_raw(pid), Signal::SIGTERM)?;

            let deadline = std::time::Instant::now() + GRACEFUL_TIMEOUT;
            loop {
                if !process_alive(pid) {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    eprintln!("  \x1b[31m✕\x1b[0m Process did not exit gracefully, forcing...");
                    signal::kill(Pid::from_raw(pid), Signal::SIGKILL)?;
                    std::thread::sleep(POLL_INTERVAL);
                    break;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            let _ = std::fs::remove_file(pid_path);
            println!("  \x1b[32m▲\x1b[0m \x1b[1mRune stopped\x1b[0m");
        } else {
            let _ = std::fs::remove_file(pid_path);
        }
    }

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon").arg("start");
    cmd.arg("--log-file").arg(&args.log_file);
    cmd.arg("--pid-file").arg(&args.pid_file);
    cmd.env("RUST_LOG", &args.log_level);

    cmd.spawn()?;

    println!("  \x1b[32m▲\x1b[0m Rune restarted with \x1b[1mRUST_LOG={}\x1b[0m", args.log_level);
    println!("    \x1b[2mLog file\x1b[0m  {}", args.log_file);
    println!();
    println!("  Run \x1b[1mrune debug\x1b[0m to tail logs.");
    println!();

    Ok(())
}

fn tail_log(log_file: &str) -> Result<()> {
    let path = Path::new(log_file);

    if !path.exists() {
        bail!(
            "log file not found: {}\n  (is the daemon running? start with: rune daemon start)",
            log_file
        );
    }

    println!("  \x1b[2mTailing\x1b[0m {log_file}  \x1b[2m(Ctrl-C to stop)\x1b[0m");
    println!();

    let status = std::process::Command::new("tail")
        .arg("-f")
        .arg(log_file)
        .status()?;

    if !status.success() {
        bail!("tail exited with status: {status}");
    }

    Ok(())
}

fn process_alive(pid: i32) -> bool {
    signal::kill(Pid::from_raw(pid), None).is_ok()
}
