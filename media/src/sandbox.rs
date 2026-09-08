use crate::{formats::Format, Error, Preview, Request, MAX_INPUT, MAX_METADATA, MAX_OUTPUT};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsRawFd,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static WORKERS: AtomicUsize = AtomicUsize::new(0);
static STAGING: AtomicUsize = AtomicUsize::new(0);
struct Permit(&'static AtomicUsize);
impl Permit {
    fn acquire(counter: &'static AtomicUsize) -> Result<Self, Error> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| Error::Limit)?;
        Ok(Self(counter))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Private, memory-backed staging. Callers must authenticate the entire file before rendering.
pub struct Input {
    file: File,
    length: u64,
    _permit: Permit,
}
impl Input {
    pub fn new() -> Result<Self, Error> {
        let permit = Permit::acquire(&STAGING)?;
        let file = tempfile::tempfile_in("/dev/shm")?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            file,
            length: 0,
            _permit: permit,
        })
    }
}
impl Write for Input {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > MAX_INPUT.saturating_sub(self.length) {
            return Err(std::io::Error::other("preview input limit"));
        }
        let count = self.file.write(bytes)?;
        self.length += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

pub struct Runtime {
    pub worker: PathBuf,
    pub pdfium: Option<PathBuf>,
    /// Directory containing LibreOffice and its shared libraries.
    pub office_libraries: Option<PathBuf>,
    pub office_language_data: Option<PathBuf>,
}
pub struct Job<'a> {
    pub runtime: &'a Runtime,
    pub request: &'a Request,
    pub cancel: &'a AtomicBool,
}
impl Job<'_> {
    pub fn render(&self, input: &Input, format: Format) -> Result<Preview, Error> {
        self.runtime
            .render(input, format, self.request, self.cancel)
    }
}
impl Runtime {
    pub fn render(
        &self,
        input: &Input,
        format: Format,
        request: &Request,
        cancel: &AtomicBool,
    ) -> Result<Preview, Error> {
        request.validate()?;
        if format == Format::Unknown {
            return Err(Error::Unsupported);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        let _permit = Permit::acquire(&WORKERS)?;
        let worker = self.worker.canonicalize().map_err(|_| Error::Unavailable)?;
        if !worker.is_file() {
            return Err(Error::Unavailable);
        }
        let mut command = Command::new("/usr/bin/bwrap");
        command
            .env_clear()
            .args([
                "--unshare-all",
                "--unshare-user",
                "--disable-userns",
                "--die-with-parent",
                "--new-session",
                "--clearenv",
                "--cap-drop",
                "ALL",
                "--ro-bind",
                "/usr",
                "/usr",
                "--symlink",
                "usr/bin",
                "/bin",
                "--symlink",
                "usr/lib",
                "/lib",
                "--symlink",
                "usr/lib",
                "/lib64",
                "--ro-bind",
            ])
            .arg(worker)
            .arg("/worker")
            .arg("--ro-bind-data")
            .arg("0")
            .arg("/input")
            .args([
                "--size",
                "268435456",
                "--tmpfs",
                "/tmp",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--ro-bind-try",
                "/etc/fonts",
                "/etc/fonts",
                "--setenv",
                "HOME",
                "/tmp",
                "--setenv",
                "PATH",
                "/usr/bin",
                "--setenv",
                "LANG",
                "C.UTF-8",
            ]);
        if let Some(pdfium) = &self.pdfium {
            command
                .arg("--ro-bind")
                .arg(pdfium.canonicalize().map_err(|_| Error::Unavailable)?)
                .args(["/libpdfium.so", "--setenv", "SIGIL_PDFIUM", "/libpdfium.so"]);
        }
        if let Some(office) = &self.office_libraries {
            command
                .arg("--ro-bind")
                .arg(office.canonicalize().map_err(|_| Error::Unavailable)?)
                .args([
                    "/opt/lib",
                    "--setenv",
                    "LD_LIBRARY_PATH",
                    "/opt/lib:/opt/lib/libreoffice/program",
                    "--setenv",
                    "SAL_USE_VCLPLUGIN",
                    "svp",
                    "--setenv",
                    "SIGIL_OFFICE",
                    "/opt/lib/libreoffice/program/soffice",
                ]);
        }
        if let Some(data) = &self.office_language_data {
            command.args([
                "--tmpfs",
                "/usr/share",
                "--ro-bind-try",
                "/usr/share/fonts",
                "/usr/share/fonts",
                "--ro-bind-try",
                "/usr/share/fontconfig",
                "/usr/share/fontconfig",
            ]);
            command
                .arg("--ro-bind")
                .arg(data.canonicalize().map_err(|_| Error::Unavailable)?)
                .arg("/usr/share/liblangtag");
        }
        command
            .args([
                "--",
                "/usr/bin/prlimit",
                "--as=2147483648",
                "--cpu=20",
                "--fsize=134217728",
                "--nofile=128",
                "--core=0",
                "--",
                "/worker",
                "/input",
            ])
            .arg(serde_json::to_string(&format).map_err(|_| Error::Invalid)?)
            .arg(serde_json::to_string(request).map_err(|_| Error::Invalid)?)
            .stdin(Stdio::from(File::open(format!(
                "/proc/self/fd/{}",
                input.file.as_raw_fd()
            ))?))
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| Error::Unavailable)?;
        let stdout = child.stdout.take().ok_or(Error::Io)?;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take((MAX_METADATA + MAX_OUTPUT + 14) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        });
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut result = None;
        let status = loop {
            if cancel.load(Ordering::Acquire) {
                break Err(Error::Cancelled);
            }
            if Instant::now() >= deadline {
                break Err(Error::Limit);
            }
            if result.is_none() {
                match receiver.try_recv() {
                    Ok(value) => {
                        if value
                            .as_ref()
                            .map_or(true, |bytes| bytes.len() > MAX_METADATA + MAX_OUTPUT + 13)
                        {
                            break Err(Error::Limit);
                        }
                        result = Some(value);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => (),
                    Err(_) => break Err(Error::Io),
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err(Error::from_exit_code(status.code()))
                    }
                }
                Ok(None) => (),
                Err(_) => break Err(Error::Io),
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if status.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        let _ = reader.join();
        status?;
        let bytes = result.unwrap_or_else(|| {
            receiver
                .recv()
                .unwrap_or_else(|_| Err(std::io::Error::other("preview output")))
        })?;
        Preview::read(bytes.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    #[test]
    fn staging_has_no_directory_entry_and_releases_its_capacity() {
        let mut input = Input::new().unwrap();
        input.write_all(b"synthetic").unwrap();
        assert_eq!(input.file.metadata().unwrap().nlink(), 0);
        assert_eq!(input.file.metadata().unwrap().mode() & 0o777, 0o600);
        let second = Input::new().unwrap();
        assert!(matches!(Input::new(), Err(Error::Limit)));
        drop(second);
        assert!(Input::new().is_ok());
    }
}
