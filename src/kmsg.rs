use std::fs::OpenOptions;
use std::io::Write;

pub fn log_to_kmsg(message: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new().write(true).open("/dev/kmsg")?;
    writeln!(f, "truthdb-installer: {message}")
}

pub fn log_to_serial(message: &str) -> std::io::Result<()> {
    // Best-effort: write diagnostics to a serial console device so we can capture logs
    // without spamming the on-screen console. Hyper-V typically uses ttyS0.
    let candidates = ["/dev/ttyS0", "/dev/ttyAMA0", "/dev/hvc0"];

    let mut last_err: Option<std::io::Error> = None;
    for dev in candidates {
        match OpenOptions::new().write(true).append(true).open(dev) {
            Ok(mut f) => {
                return writeln!(f, "truthdb-installer: {message}");
            }
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no serial device found")
    }))
}
