use std::fs;
use std::path::Path;
use nix::mount::{
    MsFlags,
    mount
};

pub fn bind_mount(path: &Path) -> Result<(), anyhow::Error> {
    // Bind mount path to itself. This is a slight trick
    // it makes path a mount point separate from its parent mount namespace
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

#[allow(unused)]
pub fn print_dir(dir: impl AsRef<Path>) -> anyhow::Result<()> {
    let dir = dir.as_ref();
    println!("Contents of {}", dir.display());
    let dir = fs::read_dir(dir)?;
    for e in dir.flatten() {
        let file_type = e.file_type()?;
        let x1 = format!("{:?}", file_type);
        let t = match file_type {
            d if d.is_dir() => "d",
            f if f.is_file() => "f",
            s if s.is_symlink() => "L",
            x => x1.as_str(),
        };

        if file_type.is_symlink() {
            let linked = fs::read_link(e.path())?;
            println!("{}    {} -> {}", t, e.path().display(), linked.display());
        } else {
            println!("{}    {}", t, e.path().display());
        }
    }

    Ok(())
}