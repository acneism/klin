//! Открыть файл в связанной программе и показать файл в Проводнике —
//! «Открыть» и «Показать в папке» в списке загрузок Firefox.
//! Через `explorer.exe`, без вызовов Win32, поэтому без unsafe.

use std::io;
use std::path::Path;
use std::process::Command;

/// Открыть файл программой по умолчанию.
pub fn open(path: &Path) -> io::Result<()> {
    if cfg!(windows) {
        Command::new("explorer").arg(path).spawn().map(drop)
    } else {
        Command::new("xdg-open").arg(path).spawn().map(drop)
    }
}

/// Открыть папку с файлом и выделить его.
pub fn show_in_folder(path: &Path) -> io::Result<()> {
    if cfg!(windows) {
        // Аргумент «/select,<путь>» — одной строкой, иначе Проводник его не разберёт.
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        Command::new("explorer").arg(arg).spawn().map(drop)
    } else {
        let dir = path.parent().unwrap_or(path);
        Command::new("xdg-open").arg(dir).spawn().map(drop)
    }
}
