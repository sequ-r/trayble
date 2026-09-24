//! Persistence of the configuration.
//!
//! One small TOML file, written only by the daemon (the settings app goes
//! through its D-Bus API), so there is exactly one source of truth.

use std::path::PathBuf;

use taskbar_core::Config;

/// Directory of the config file, following the XDG base directory spec.
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("taskbar")
}

/// The config file itself.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Read the config, falling back to [`Config::default`] when the file is
/// missing or unreadable: a broken config must never keep the tray from
/// starting.
pub fn load(path: &std::path::Path) -> Config {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Config::default();
    };
    toml::from_str::<Config>(&text)
        .map(Config::normalized)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, path = %path.display(), "ignoring broken config");
            Config::default()
        })
}

/// Write the config atomically: a crash must not leave a half written file.
pub fn save(path: &std::path::Path, config: &Config) -> std::io::Result<()> {
    let text = toml::to_string_pretty(config).expect("config serialises");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("toml.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("taskbar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");

        let config = Config::default().with_hidden("steam", true).with_order(vec!["b".into()]);
        save(&path, &config).expect("save");
        assert_eq!(load(&path), config);

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn missing_and_broken_configs_fall_back_to_defaults() {
        let missing = std::path::Path::new("/nonexistent/taskbar/config.toml");
        assert_eq!(load(missing), Config::default());

        let dir = std::env::temp_dir().join(format!("taskbar-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, "not = [valid").expect("write");
        assert_eq!(load(&path), Config::default());

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
