//! Out-of-band pixel transfer for terminals on the same machine: kitty's temp-file (`t=t`)
//! and shared-memory (`t=s`) media. Writing the pixels here and sending only their location
//! skips base64 and the PTY, which matters for animation and hover redraws.
//!
//! The terminal deletes (unlinks) what it reads. If it doesn't read it (it doesn't support the
//! medium), call [`discard`].

use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::kitty::Medium;

fn unique() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    format!("{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// Write `bytes` to a new temporary file (mode 0600) whose name kitty accepts for `t=t`.
pub fn temp_file(bytes: &[u8]) -> io::Result<Medium> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = std::env::temp_dir().join(format!("tty-graphics-protocol-kittychart-{}.rgba", unique()));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    f.write_all(bytes)?;
    Ok(Medium::TempFile(path.to_string_lossy().into_owned()))
}

/// Write `bytes` to a new POSIX shared-memory object (Linux; macOS can't `write` to one).
#[cfg(target_os = "linux")]
pub fn shared_memory(bytes: &[u8]) -> io::Result<Medium> {
    use rustix::fs::Mode;
    use rustix::shm::OFlags;
    let name = format!("/kittychart-{}", unique());
    let fd = rustix::shm::open(&name, OFlags::CREATE | OFlags::EXCL | OFlags::RDWR, Mode::RUSR | Mode::WUSR)?;
    let mut f = std::fs::File::from(fd);
    f.write_all(bytes)?;
    Ok(Medium::SharedMemory(name))
}

/// Shared memory isn't writable without `mmap` here.
#[cfg(not(target_os = "linux"))]
pub fn shared_memory(_bytes: &[u8]) -> io::Result<Medium> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "shared-memory transmission needs Linux"))
}

/// Write `bytes` for transmission over `kind` (a [`Medium`] whose name is ignored).
pub fn write(kind: &Medium, bytes: &[u8]) -> io::Result<Medium> {
    match kind {
        Medium::Direct => Ok(Medium::Direct),
        Medium::TempFile(_) => temp_file(bytes),
        Medium::SharedMemory(_) => shared_memory(bytes),
    }
}

/// Remove a file or shared-memory object the terminal didn't consume.
pub fn discard(m: &Medium) {
    match m {
        Medium::Direct => {}
        Medium::TempFile(path) => {
            let _ = std::fs::remove_file(path);
        }
        Medium::SharedMemory(name) => {
            #[cfg(target_os = "linux")]
            let _ = rustix::shm::unlink(name.as_str());
            #[cfg(not(target_os = "linux"))]
            let _ = name;
        }
    }
}

/// Whether the terminal consumed it already (deleted the file / unlinked the object).
pub fn consumed(m: &Medium) -> bool {
    match m {
        Medium::Direct => true,
        Medium::TempFile(path) => !std::path::Path::new(path).exists(),
        Medium::SharedMemory(name) => {
            #[cfg(target_os = "linux")]
            {
                !std::path::Path::new("/dev/shm").join(name.trim_start_matches('/')).exists()
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = name;
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_files_and_shared_memory_hold_the_bytes() {
        let m = temp_file(b"abcd").unwrap();
        let Medium::TempFile(path) = &m else { panic!() };
        assert!(path.contains("tty-graphics-protocol"));
        assert_eq!(std::fs::read(path).unwrap(), b"abcd");
        assert!(!consumed(&m));
        discard(&m);
        assert!(consumed(&m));
        #[cfg(target_os = "linux")]
        {
            let m = shared_memory(b"wxyz").unwrap();
            let Medium::SharedMemory(name) = &m else { panic!() };
            assert_eq!(std::fs::read(format!("/dev/shm{name}")).unwrap(), b"wxyz");
            discard(&m);
            assert!(consumed(&m));
        }
    }
}
