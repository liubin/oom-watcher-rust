use eventfd::{eventfd, EfdFlags};
use nix::sys::eventfd;
use nix::unistd;
use std::env;
use std::fs::{self, File};
use std::path::Path;
use std::{
    fmt, io,
    io::{Read, Result, Write},
    mem,
    os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::AsyncReadExt;

use futures::ready;
use tokio::io::{unix::AsyncFd, AsyncRead, AsyncWrite, ReadBuf};

const EVENT_NAME:&str = "memory.oom_control";
     
fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        println!("USAGE: {} sync/async <cg_dir>", args[0]);
        std::process::exit(1);
    }
    let mode = args[1].as_str();
    if mode == "sync" {
        sync_fn(args[2].as_str())
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            async_fn(args[2].as_str()).await;
        });
    }
}

async fn async_fn(cg_dir: &str) {

    let path = Path::new(&cg_dir).join(EVENT_NAME);
    let event_file = File::open(path.clone()).unwrap();
    let eventfd = eventfd(0, EfdFlags::EFD_CLOEXEC).unwrap();
    let event_control_path = Path::new(&cg_dir).join("cgroup.event_control");
    let data = format!("{} {}", eventfd, event_file.as_raw_fd());
    fs::write(&event_control_path, data).unwrap();

    let mut eventfd_stream = unsafe { PipeStream::from_raw_fd(eventfd) };

    let _ = tokio::spawn(async move {
        loop {
            let mut buf = [0u8; 8];
            match eventfd_stream.read(&mut buf).await {
                Err(err) => {
                    println!("err: {:?}", err);

                    break;
                }
                Ok(s) => {
                    println!("ok: {:?}", s);
                }
            }

            // When a cgroup is destroyed, an event is sent to eventfd.
            // So if the control path is gone, return instead of notifying.
            if !Path::new(&event_control_path).exists() {
                println!("deleted: {:?}", &event_control_path);
                break;
            }
        }

        println!("finished");
    })
    .await;
}

fn sync_fn(cg_dir: &str) {
    let path = Path::new(cg_dir).join(EVENT_NAME);
    let event_file = File::open(path).unwrap();
    let eventfd = eventfd(0, EfdFlags::EFD_CLOEXEC).unwrap();
    let event_control_path = Path::new(cg_dir).join("cgroup.event_control");
    let data = format!("{} {}", eventfd, event_file.as_raw_fd());
    fs::write(&event_control_path, data).unwrap();

    let mut eventfd_file = unsafe { File::from_raw_fd(eventfd) };

    loop {
        let mut buf = [0; 8];
        match eventfd_file.read(&mut buf) {
            Err(err) => {
                println!("failed to read eventfd: {:?}", err);
            }
            Ok(_) => {
                println!("OOM for {:?}", cg_dir);
            }
        }

        // When a cgroup is destroyed, an event is sent to eventfd.
        // So if the control path is gone, return instead of notifying.
        if !Path::new(&event_control_path).exists() {
            break;
        }
    }

    println!("finished");
}

////////////////////
// from https://github.com/kata-containers/kata-containers/blob/main/src/agent/rustjail/src/cgroups/notifier.rs

fn set_nonblocking(fd: RawFd) {
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
    }
}

struct StreamFd(RawFd);

impl io::Read for &StreamFd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match unistd::read(self.0, buf) {
            Ok(l) => Ok(l),
            Err(e) => Err(e.as_errno().unwrap().into()),
        }
    }
}

impl io::Write for &StreamFd {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match unistd::write(self.0, buf) {
            Ok(l) => Ok(l),
            Err(e) => Err(e.as_errno().unwrap().into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl StreamFd {
    fn close(&mut self) -> io::Result<()> {
        match unistd::close(self.0) {
            Ok(()) => Ok(()),
            Err(e) => Err(e.as_errno().unwrap().into()),
        }
    }
}

impl Drop for StreamFd {
    fn drop(&mut self) {
        self.close().ok();
    }
}

impl AsRawFd for StreamFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

pub struct PipeStream(AsyncFd<StreamFd>);

impl PipeStream {
    pub fn new(fd: RawFd) -> Result<Self> {
        set_nonblocking(fd);
        Ok(Self(AsyncFd::new(StreamFd(fd))?))
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        self.0.get_mut().close()
    }

    pub fn from_fd(fd: RawFd) -> Self {
        unsafe { Self::from_raw_fd(fd) }
    }
}

impl AsRawFd for PipeStream {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl IntoRawFd for PipeStream {
    fn into_raw_fd(self) -> RawFd {
        let fd = self.as_raw_fd();
        mem::forget(self);
        fd
    }
}

impl FromRawFd for PipeStream {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new(fd).unwrap()
    }
}

impl fmt::Debug for PipeStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PipeStream({})", self.as_raw_fd())
    }
}

impl AsyncRead for PipeStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        let b;
        unsafe {
            b = &mut *(buf.unfilled_mut() as *mut [mem::MaybeUninit<u8>] as *mut [u8]);
        };

        loop {
            let mut guard = ready!(self.0.poll_read_ready(cx))?;

            match guard.try_io(|inner| inner.get_ref().read(b)) {
                Ok(Ok(n)) => {
                    unsafe {
                        buf.assume_init(n);
                    }
                    buf.advance(n);
                    return Ok(()).into();
                }
                Ok(Err(e)) => return Err(e).into(),
                Err(_would_block) => {
                    continue;
                }
            }
        }
    }
}

impl AsyncWrite for PipeStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = ready!(self.0.poll_write_ready(cx))?;

            match guard.try_io(|inner| inner.get_ref().write(buf)) {
                Ok(result) => return Poll::Ready(result),
                Err(_would_block) => continue,
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().shutdown()?;
        Poll::Ready(Ok(()))
    }
}
