use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub positions: BTreeMap<PathBuf, f64>,
    #[serde(default)]
    pub last: Option<PathBuf>,
    #[serde(default)]
    pub library: Option<PathBuf>,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default = "default_volume")]
    pub volume: f32,
}
fn default_speed() -> f32 {
    1.0
}
fn default_volume() -> f32 {
    0.8
}
impl Default for State {
    fn default() -> Self {
        Self {
            positions: BTreeMap::new(),
            last: None,
            library: None,
            speed: default_speed(),
            volume: default_volume(),
        }
    }
}
impl State {
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(&serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
            file.sync_all()?;
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
pub fn location() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_STATE_HOME").filter(|s| Path::new(s).is_absolute()) {
        return Ok(PathBuf::from(path).join("cozyread/progress.json"));
    }
    env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".local/state/cozyread/progress.json"))
        .ok_or_else(|| io::Error::other("Set HOME or XDG_STATE_HOME to save listening progress"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_roundtrip_and_corruption_is_reported() {
        let dir = env::temp_dir().join(format!("cozyread-state-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.json");
        let mut state = State::default();
        let book = PathBuf::from("/books/a book.m4b");
        state.positions.insert(book.clone(), 1234.5);
        state.last = Some(book.clone());
        state.library = Some(PathBuf::from("/books"));
        state.speed = 1.7;
        state.volume = 0.45;
        state.save(&path).unwrap();
        let restored = State::load(&path).unwrap();
        assert_eq!(restored.positions[&book], 1234.5);
        assert_eq!(restored.last, Some(book));
        assert_eq!(restored.library, Some(PathBuf::from("/books")));
        assert_eq!(restored.speed, 1.7);
        assert_eq!(restored.volume, 0.45);
        fs::write(&path, b"broken").unwrap();
        assert!(State::load(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn old_progress_files_get_default_preferences() {
        let state: State =
            serde_json::from_str(r#"{"positions":{"/book.m4b":93.25},"last":"/book.m4b"}"#)
                .unwrap();
        assert_eq!(state.speed, 1.0);
        assert_eq!(state.volume, 0.8);
        assert_eq!(state.positions[&PathBuf::from("/book.m4b")], 93.25);
        assert!(state.library.is_none());
    }
}
