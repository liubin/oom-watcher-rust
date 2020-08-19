use eventfd::{eventfd, EfdFlags};
use nix::sys::eventfd;
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        println!("USAGE: {} <cg_dir>", args[0]);
        std::process::exit(1);
    }

    let cg_dir = args[1].as_str();
    let event_name = "memory.oom_control";

    let path = Path::new(cg_dir).join(event_name);
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
