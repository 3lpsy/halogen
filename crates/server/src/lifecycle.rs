use std::process;
use tracing::error;

/// Replace the current process image with a fresh invocation of this binary
/// (same argv + inherited env), so the new process re-runs `Config::resolve` and
/// applies the (possibly just-written) overrides file. `execv` only returns on
/// failure.
#[cfg(unix)]
pub(crate) fn re_exec() -> ! {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        error!("re-exec: cannot determine current executable: {e}");
        process::exit(1);
    });
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let err = std::process::Command::new(exe).args(args).exec();
    error!("re-exec failed: {err}");
    process::exit(1);
}

#[cfg(not(unix))]
pub(crate) fn re_exec() -> ! {
    error!("Restart via re-exec is only supported on Unix; exiting instead");
    process::exit(0);
}
