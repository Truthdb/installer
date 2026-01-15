use std::fs::OpenOptions;
use std::io::Write;

pub fn log_to_kmsg(message: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new().write(true).open("/dev/kmsg")?;
    writeln!(f, "truthdb-installer: {message}")
}
