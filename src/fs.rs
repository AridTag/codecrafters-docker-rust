use std::path::Path;
use nix::mount::{
    MsFlags,
    mount
};

pub fn bind_mount(path: &Path) -> Result<(), anyhow::Error> {
    // Bind mount new_root to itself. This is a slight trick
    // it makes new_root a mount point separate from its parent mount namespace
    match mount(
        Some(path),
        path,
        Some("none"),
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&'_ str>
    ) {
        Err(e) => Err(e.into()),

        _ => Ok(())
    }
}