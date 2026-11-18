//! Windows Shell **classic** context menu bridge.
//! Always start in a dedicated child process because third-party in-proc COM
//! handlers cannot be made memory-safe by Rust and may crash or hang.
use anyhow::{anyhow, Context, Result};
use std::{path::Path, process::Command};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ShellResult { ExecutedOrDismissed, MarkSensitive, ViewHistory, DocumentTask, OpenExplorer }

#[cfg(windows)]
extern "C" { fn ladon_show_shell_menu(path: *const u16) -> i32; }

pub fn run_isolated_host(path: &Path) -> Result<ShellResult> {
    #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;
        let wide:Vec<u16>=path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let result = unsafe { ladon_show_shell_menu(wide.as_ptr()) };
        return match result {
            0=>Ok(ShellResult::ExecutedOrDismissed),
            0x7001=>Ok(ShellResult::MarkSensitive),
            0x7002=>Ok(ShellResult::ViewHistory),
            0x7003=>Ok(ShellResult::DocumentTask),
            0x7004=>Ok(ShellResult::OpenExplorer),
            x=>Err(anyhow!("Windows classic Shell menu failed ({x}); Explorer fallback required")),
        };
    }
    #[cfg(not(windows))] {
        let _ = path;
        Err(anyhow!("Windows Shell bridge is available on Windows only"))
    }
}

pub fn spawn_shell_host(root:&Path,path:&Path) -> Result<i32> {
    let exe=std::env::current_exe().context("unable to find executable")?;
    let status=Command::new(exe).arg("--root").arg(root)
        .arg("shell-host-internal").arg(path).status()?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(windows)]
pub fn open_in_explorer(path:&Path)->Result<()> {
    let argument=format!("/select,\"{}\"",path.to_string_lossy());
    Command::new("explorer.exe").arg(argument).spawn()?;
    Ok(())
}
#[cfg(not(windows))]
pub fn open_in_explorer(_path:&Path)->Result<()> {Err(anyhow!("Windows only"))}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ut_shell_result_distinct(){ assert_ne!(ShellResult::MarkSensitive, ShellResult::ViewHistory); }
}
