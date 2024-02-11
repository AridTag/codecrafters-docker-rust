mod sandbox;
mod fs;

use anyhow::Result;
use nix::sys::wait::waitpid;
use crate::sandbox::Sandbox;

// Usage: your_docker.sh run <image> <command> <arg1> <arg2> ...
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let command = &args[3];
    let command_args = &args[4..];
    match Sandbox::run(command, command_args) {
        Ok(sandbox) => {
            let output = sandbox.consume_output();
            waitpid(sandbox.child_pid, None).expect("Failed to wait for child process");

            std::process::exit(output.status);
        }

        Err(e) => {
            eprintln!("Failed to run Sandbox - {:?}", e);
            std::process::exit(-1);
        }
    }
}
