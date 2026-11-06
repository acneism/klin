use std::io;
use std::path::Path;
use std::process::Command;

pub fn open(path: &Path) -> io::Result<()> {
    if cfg!(windows) {
        Command::new("explorer").arg(path).spawn().map(drop)
    } else {
        Command::new("xdg-open").arg(path).spawn().map(drop)
    }
}

pub fn show_in_folder(path: &Path) -> io::Result<()> {
    if cfg!(windows) {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        Command::new("explorer").arg(arg).spawn().map(drop)
    } else {
        let dir = path.parent().unwrap_or(path);
        Command::new("xdg-open").arg(dir).spawn().map(drop)
    }
}
