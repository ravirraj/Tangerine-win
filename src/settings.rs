//! Application settings and persistent configuration.
//!
//! Stores user preferences in `%APPDATA%\Open Tangerine\settings.json`:
//! - Appearance: Theme (System, Dark, Light), Accent color
//! - Destination: Beside original or custom output folder
//! - System: Sound on completion, Launch on startup, Tray icon, Shell context menu

use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeMode {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "dark" => ThemeMode::Dark,
            "light" => ThemeMode::Light,
            _ => ThemeMode::System,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            ThemeMode::System => "System",
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: ThemeMode,
    pub accent: String,
    pub output_beside_original: bool,
    pub custom_output_dir: Option<PathBuf>,
    pub sound_enabled: bool,
    pub launch_at_startup: bool,
    pub show_tray_icon: bool,
    pub shell_menu_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            accent: "#FF5C00".to_string(),
            output_beside_original: true,
            custom_output_dir: None,
            sound_enabled: true,
            launch_at_startup: false,
            show_tray_icon: true,
            shell_menu_enabled: true,
        }
    }
}

impl AppSettings {
    /// Returns default path for settings file: `%APPDATA%\Open Tangerine\settings.json`.
    pub fn default_path() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("Open Tangerine").join("settings.json")
        } else {
            PathBuf::from("settings.json")
        }
    }

    /// Loads settings from default path, or returns default settings if file missing.
    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    /// Loads settings from a specific file path.
    pub fn load_from(path: &Path) -> Self {
        if let Ok(data) = fs::read_to_string(path) {
            if let Ok(settings) = serde_json::from_str(&data) {
                return settings;
            }
        }
        Self::default()
    }

    /// Saves settings to default path.
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::default_path())
    }

    /// Saves settings to a specific file path.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_mode_parsing_and_titles() {
        assert_eq!(ThemeMode::parse("dark"), ThemeMode::Dark);
        assert_eq!(ThemeMode::parse("light"), ThemeMode::Light);
        assert_eq!(ThemeMode::parse("system"), ThemeMode::System);
        assert_eq!(ThemeMode::parse("anything"), ThemeMode::System);

        assert_eq!(ThemeMode::System.title(), "System");
        assert_eq!(ThemeMode::Dark.title(), "Dark");
        assert_eq!(ThemeMode::Light.title(), "Light");
    }

    #[test]
    fn default_settings_have_sane_values() {
        let s = AppSettings::default();
        assert_eq!(s.theme, ThemeMode::System);
        assert_eq!(s.accent, "#FF5C00");
        assert!(s.output_beside_original);
        assert!(s.sound_enabled);
        assert!(s.show_tray_icon);
    }

    #[test]
    fn settings_save_and_load_roundtrips() {
        let temp_dir = std::env::temp_dir().join(format!("ot-settings-{}", std::process::id()));
        let path = temp_dir.join("settings.json");

        let mut s = AppSettings::default();
        s.theme = ThemeMode::Dark;
        s.accent = "#007AFF".to_string();
        s.sound_enabled = false;
        s.custom_output_dir = Some(PathBuf::from(r"C:\Exports"));

        assert!(s.save_to(&path).is_ok());
        assert!(path.is_file());

        let loaded = AppSettings::load_from(&path);
        assert_eq!(loaded, s);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
