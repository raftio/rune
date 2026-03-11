use anyhow::Result;
use nix::unistd::Pid;
use std::path::Path;

use crate::cli::DaemonStatusArgs;

pub fn exec(args: DaemonStatusArgs) -> Result<()> {
    let pid_path = Path::new(&args.pid_file);

    println!();
    if !pid_path.exists() {
        println!("  \x1b[33m●\x1b[0m \x1b[1mDaemon status\x1b[0m \x1b[2m(not running)\x1b[0m");
        println!();
        println!("    No PID file at {} — daemon is not running.", args.pid_file);
        println!();
        return Ok(());
    }

    let raw = std::fs::read_to_string(pid_path)?;
    let pid: i32 = raw.trim().parse().map_err(|_| {
        anyhow::anyhow!("invalid pid in {}: {:?}", args.pid_file, raw.trim())
    })?;

    let alive = nix::sys::signal::kill(Pid::from_raw(pid), None).is_ok();
    if alive {
        println!("  \x1b[32m▲\x1b[0m \x1b[1mDaemon status\x1b[0m \x1b[2m(running)\x1b[0m");
        println!();
        println!("    PID file: {}", args.pid_file);
        println!("    PID:      {pid}");
        println!();
    } else {
        println!("  \x1b[33m●\x1b[0m \x1b[1mDaemon status\x1b[0m \x1b[2m(stopped)\x1b[0m");
        println!();
        println!("    Stale PID file at {} — process {pid} no longer running.", args.pid_file);
        println!();
    }
    Ok(())
}
