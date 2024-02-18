use std::fs;
use std::fs::File;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use anyhow::{Context, Result};
use nix::mount::{MntFlags, umount2};
use nix::sched::{CloneFlags, unshare};
use nix::sys::stat::{makedev, mknod, Mode, SFlag};
use nix::unistd::{close, fork, ForkResult, Pid, pivot_root, read, write};
use serde::{Deserialize, Serialize};
use tempfile::{tempdir, TempDir};
use crate::fs::bind_mount;
use crate::images::DockerRegistryClient;

const OLD_ROOT: &str = "old_root";

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
    pub async fn run(image: &str, command: &str, args: &[String]) -> Result<Sandbox> {
        let (r_pipe, w_pipe) = nix::unistd::pipe().expect("Failed to create pipe");
        let (image_name, image_tag) = {
            let image_tag: Vec<_> = image.split(':').collect();
            let name = image_tag[0];
            let tag = match image_tag.get(1) {
                Some(tag) => *tag,
                _ => "latest"
            };
            (name, tag)
        };

        let StartupParams { new_root} = init_sandbox_root(image_name, image_tag).await?;

        let child_pid: Pid = unsafe {
            match fork()? {
                ForkResult::Parent { child } => {
                    close(w_pipe).expect("Failed to close write pipe in parent");

                    child
                },
                ForkResult::Child => {
                    close(r_pipe).expect("Failed to close read pipe in child");

                    unshare(CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWPID)
                        .expect("Failed to unshare mount and pid namespaces");
                    
                    pivot_root(new_root.path(), &new_root.path().join(OLD_ROOT))?;
                    let old_root = PathBuf::from("/").join(OLD_ROOT);
                    _ = umount2(old_root.as_path(), MntFlags::MNT_DETACH);
                    fs::remove_dir_all(old_root.as_path()).expect("Failed to remove old root dir");
                    std::env::set_current_dir("/").expect("Failed to set current dir");

                    let output = Command::new(command)
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
}

async fn init_sandbox_root(image_name: &str, image_tag: &str) -> Result<StartupParams> {
    let new_root = tempdir().expect("Failed to create new tmp root dir");
    let new_root_path = new_root.path();
    bind_mount(new_root_path)?;

    create_dev_null(new_root_path);

    fs::create_dir_all(new_root.path().join("tmp/")).expect("Failed to create tmp directory");

    {
        let dir = new_root_path.join(OLD_ROOT);
        _ = umount2(&dir, MntFlags::MNT_DETACH); // Just in case

        fs::create_dir_all(&dir)
            .context(format!("Failed to create dir '{}'", dir.to_string_lossy()))?;
    }

    let layer_archives = pull_image_layers(image_name, image_tag, "/tmp/").await?;
    extract_layers(&layer_archives, new_root_path).await?;
    for archive in &layer_archives {
        fs::remove_file(archive).expect("Failed to remove layer archive");
    }

    Ok(StartupParams {
        new_root,
    })
}

async fn pull_image_layers(image_name: &str, image_tag: &str, dest_dir: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let dest_dir = dest_dir.as_ref().to_path_buf();

    //println!("Pulling image {image_name}:{image_tag}");

    let mut layer_archives = Vec::<PathBuf>::new();
    let mut client = DockerRegistryClient::for_image(image_name, image_tag);
    let manifest = client.get_manifest().await?;
    for layer in &manifest.layers {
        let split_pos = layer.digest.find(':').expect("Can't split digest hash?");
        let layer_hash = &layer.digest[(split_pos + 1)..];
        let dest_file = dest_dir.join(layer_hash);
        client.download_layer(&dest_file, &layer.digest, &layer.media_type).await?;
        layer_archives.push(dest_file);
    }

    Ok(layer_archives)
}

async fn extract_layers(archives: &[PathBuf], dest: impl AsRef<Path>) -> Result<()> {
    //println!("Extracting layers...");
    for archive in archives {
        let tar_gz = File::open(archive)?;
        let tar = flate2::read::GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(tar);
        archive.unpack(dest.as_ref())?;
    }

    Ok(())
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