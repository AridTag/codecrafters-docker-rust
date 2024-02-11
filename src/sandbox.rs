use std::fs;
use std::fs::copy;
use std::os::fd::RawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use anyhow::{Context, Result};
use nix::mount::{MntFlags, umount2};
use nix::sched::{CloneFlags, unshare};
use nix::sys::stat::{makedev, mknod, Mode, SFlag};
use nix::unistd::{chdir, close, fork, ForkResult, Pid, pivot_root, read, write};
use serde::{Deserialize, Serialize};
use tempfile::{tempdir, TempDir};
use crate::fs::bind_mount;

static OLD_ROOT: &str = "old_root";

pub struct Sandbox {
    #[allow(unused)]
    root_dir: TempDir,

    pub child_pid: Pid,
    read_pipe: RawFd
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChildStatus {
    pub status: i32,
}

impl Sandbox {
    pub fn run(command: &String, args: &[String]) -> Result<Sandbox> {
        let (r_pipe, w_pipe) = nix::unistd::pipe().expect("Failed to create pipe");
        let StartupParams { new_root, rooted_command } = init_sandbox_root(command)?;

        let child_pid: Pid = unsafe {
            match fork()? {
                ForkResult::Parent { child } => {
                    close(w_pipe).expect("Failed to close write pipe in parent");

                    child
                },
                ForkResult::Child => {
                    close(r_pipe).expect("Failed to close read pipe in child");

                    unshare(CloneFlags::CLONE_NEWNS).expect("Failed to unshare mount namespace");
                    pivot_root(new_root.path(), &new_root.path().join(OLD_ROOT))?;
                    chdir("/")?;

                    let output = Command::new(rooted_command)
                        .args(args)
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::inherit())
                        .output()
                        .with_context(|| {
                            format!(
                                "Tried to run '{}' with arguments {:?}",
                                command, args
                            )
                        })?;

                    let serializable_output = ChildStatus {
                        status: output.status.code().unwrap_or(-1),
                    };

                    let serialized_output = serde_json::to_string(&serializable_output)
                        .expect("Failed to serialize output");

                    write(w_pipe, serialized_output.as_bytes()).expect("Child failed to write to pipe");
                    close(w_pipe).expect("Child failed to close write pipe");

                    std::process::exit(0);
                }
            }
        };

        Ok(Sandbox {
            root_dir: new_root,
            child_pid,
            read_pipe: r_pipe
        })
    }

    pub fn consume_output(&self) -> ChildStatus {
        let mut read_buffer: Vec<u8> = vec![0; 1024];
        let bytes_read = read(self.read_pipe, &mut read_buffer).expect("Failed to read from pipe");
        close(self.read_pipe).expect("Parent failed to close read pipe");

        let received_output: ChildStatus = serde_json::from_slice(&read_buffer[..bytes_read])
            .expect("Failed to deserialize output");

        received_output
    }
}

struct StartupParams {
    new_root: TempDir,
    rooted_command: String,
}

fn init_sandbox_root(command: &String) -> Result<StartupParams> {
    let new_root = tempdir().expect("Failed to create new tmp root dir");
    let new_root_path = new_root.path();
    bind_mount(new_root_path)?;

    create_dev_null(new_root_path);

    let command_path = Path::new(command);
    let command_filename = command_path.file_name().expect("command is missing filename?");
    let dest = new_root_path.join(command_filename);
    copy(command_path, dest).context("Failed to copy to new root")?;

    let rooted_command = Path::new("/").join(command_filename).to_string_lossy().into();

    {
        let dir = new_root_path.join(OLD_ROOT);
        _ = umount2(&dir, MntFlags::MNT_DETACH);

        fs::create_dir_all(&dir)
            .context(format!("Failed to create dir '{}'", dir.to_string_lossy()))?;
    }

    Ok(StartupParams {
        new_root,
        rooted_command,
    })
}

fn create_dev_null(root_path: impl AsRef<Path>) {
    let root_dev = root_path.as_ref().join("dev");
    fs::create_dir_all(root_dev.clone()).expect("Failed to create dev directory");

    let dev_null = root_dev.join("null");
    mknod(
        dev_null.as_path(),
        SFlag::S_IFCHR,
        Mode::from_bits_truncate(666),
        makedev(1, 3)
    ).expect("Failed to create null device");
}

#[allow(unused)]
fn list_dir(dir: impl AsRef<Path>) -> Result<()> {
    let dir = dir.as_ref();
    println!("{}", dir.to_string_lossy());
    let dir = fs::read_dir(dir)?;
    for e in dir.flatten() {
        let t = match e.file_type()? {
            d if d.is_dir() => "Dir",
            f if f.is_file() => "File",
            _ => "wat?"
        };
        println!("{}: {}", e.path().to_string_lossy(), t);
    }

    Ok(())
}