mod commands;
mod key_container;
mod password;

use std::{env, ffi::OsString, process::ExitCode};

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|_| {
        eprintln!("M2ShelfPortableKeyTool: internal failure; no secret data was printed.");
    }));
    match commands::run(env::args_os().skip(1).collect::<Vec<OsString>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("M2ShelfPortableKeyTool: {error}");
            ExitCode::from(2)
        }
    }
}
