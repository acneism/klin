mod ini;

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub use ini::Ini;

pub const APP_DIR: &str = "Klin";

pub const DEFAULT_PROFILE_NAME: &str = "default-release";

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("профиль {0} уже используется другим процессом Klin")]
    Locked(PathBuf),
    #[error("не найдена папка для профиля: {0}")]
    NoHome(&'static str),
    #[error("ошибка ввода-вывода в {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ProfileError + '_ {
    move |source| ProfileError::Io {
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug)]
pub struct Profile {
    pub dir: PathBuf,
    pub cache_dir: PathBuf,
    _lock: File,
}

impl Profile {
    pub fn open(explicit: Option<PathBuf>) -> Result<Self, ProfileError> {
        let (dir, cache_dir) = match explicit {
            Some(dir) => {
                let cache = dir.join("cache2");
                (dir, cache)
            }
            None => {
                let roaming = roaming_base()?;
                let local = local_base()?;
                let relative = default_profile(&roaming)?;
                (roaming.join(&relative), local.join(&relative))
            }
        };
        Self::open_dirs(dir, cache_dir)
    }

    fn open_dirs(dir: PathBuf, cache_dir: PathBuf) -> Result<Self, ProfileError> {
        let fresh = !dir.exists();
        fs::create_dir_all(&dir).map_err(io_err(&dir))?;
        fs::create_dir_all(&cache_dir).map_err(io_err(&cache_dir))?;
        let lock_path = dir.join("parent.lock");
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io_err(&lock_path))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(ProfileError::Locked(dir)),
            Err(fs::TryLockError::Error(e)) => return Err(io_err(&lock_path)(e)),
        }
        if fresh {
            write_times_json(&dir)?;
        }
        log::info!("profile: {}", dir.display());
        Ok(Self {
            dir,
            cache_dir,
            _lock: lock,
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

fn write_times_json(dir: &Path) -> Result<(), ProfileError> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join("times.json");
    fs::write(&path, format!("{{\"created\":{ms},\"firstUse\":null}}")).map_err(io_err(&path))
}

#[cfg(windows)]
fn roaming_base() -> Result<PathBuf, ProfileError> {
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join(APP_DIR))
        .ok_or(ProfileError::NoHome("APPDATA"))
}

#[cfg(windows)]
fn local_base() -> Result<PathBuf, ProfileError> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join(APP_DIR))
        .ok_or(ProfileError::NoHome("LOCALAPPDATA"))
}

#[cfg(not(windows))]
fn roaming_base() -> Result<PathBuf, ProfileError> {
    std::env::var_os("HOME")
        .map(|p| PathBuf::from(p).join(".klin"))
        .ok_or(ProfileError::NoHome("HOME"))
}

#[cfg(not(windows))]
fn local_base() -> Result<PathBuf, ProfileError> {
    std::env::var_os("HOME")
        .map(|p| PathBuf::from(p).join(".cache").join("klin"))
        .ok_or(ProfileError::NoHome("HOME"))
}

pub fn default_profile(base: &Path) -> Result<PathBuf, ProfileError> {
    let ini_path = base.join("profiles.ini");
    let mut ini = match fs::read_to_string(&ini_path) {
        Ok(text) => Ini::parse(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ini::default(),
        Err(e) => return Err(io_err(&ini_path)(e)),
    };
    let sections: Vec<String> = ini
        .sections()
        .filter(|s| s.starts_with("Profile"))
        .map(str::to_owned)
        .collect();
    let chosen = sections
        .iter()
        .find(|s| ini.get(s, "Default") == Some("1"))
        .or_else(|| sections.first());
    if let Some(section) = chosen
        && let Some(path) = ini.get(section, "Path")
    {
        let path = path.replace('\\', "/");
        return Ok(if ini.get(section, "IsRelative") == Some("0") {
            PathBuf::from(path)
        } else {
            path.split('/').collect()
        });
    }

    let folder = format!("{}.{DEFAULT_PROFILE_NAME}", random_salt());
    let section = format!("Profile{}", sections.len());
    ini.set(&section, "Name", DEFAULT_PROFILE_NAME);
    ini.set(&section, "IsRelative", "1");
    ini.set(&section, "Path", &format!("Profiles/{folder}"));
    ini.set(&section, "Default", "1");
    ini.set("General", "StartWithLastProfile", "1");
    ini.set("General", "Version", "2");
    fs::create_dir_all(base).map_err(io_err(base))?;

    let tmp = base.join("profiles.ini.tmp");
    fs::write(&tmp, ini.to_string()).map_err(io_err(&tmp))?;
    fs::rename(&tmp, &ini_path).map_err(io_err(&ini_path))?;
    Ok(["Profiles", folder.as_str()].iter().collect())
}

fn random_salt() -> String {
    use std::hash::{BuildHasher, Hasher};
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    hasher.write_u32(std::process::id());
    let mut n = hasher.finish();
    (0..8)
        .map(|_| {
            let c = ALPHABET[(n % ALPHABET.len() as u64) as usize];
            n /= ALPHABET.len() as u64;
            char::from(c)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("klin-profile-test-{name}-{}", random_salt()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn creates_default_profile_and_reuses_it() {
        let base = temp("ini");
        let first = default_profile(&base).unwrap();
        let name = first.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(first.starts_with("Profiles"));
        assert_eq!(name.len(), 8 + 1 + DEFAULT_PROFILE_NAME.len());
        assert!(name.ends_with(".default-release"));
        let ini = fs::read_to_string(base.join("profiles.ini")).unwrap();
        assert!(ini.contains("[Profile0]"));
        assert!(ini.contains("Path=Profiles/"));
        assert!(ini.contains("Default=1"));
        assert_eq!(default_profile(&base).unwrap(), first);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn reads_firefox_style_ini() {
        let base = temp("ff");
        fs::create_dir_all(&base).unwrap();
        fs::write(
            base.join("profiles.ini"),
            "[Profile1]\r\nName=other\r\nIsRelative=1\r\nPath=Profiles/b.other\r\n\r\n\
             [Profile0]\r\nName=default-release\r\nIsRelative=1\r\nPath=Profiles/a.default-release\r\nDefault=1\r\n",
        )
        .unwrap();
        assert_eq!(
            default_profile(&base).unwrap(),
            PathBuf::from("Profiles").join("a.default-release")
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn second_open_is_locked() {
        let dir = temp("lock");
        let profile = Profile::open(Some(dir.clone())).unwrap();
        assert!(profile.path("times.json").exists());
        assert!(matches!(
            Profile::open(Some(dir.clone())),
            Err(ProfileError::Locked(_))
        ));
        drop(profile);
        let again = Profile::open(Some(dir.clone())).unwrap();
        drop(again);
        fs::remove_dir_all(dir).unwrap();
    }
}
