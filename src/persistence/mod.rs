pub mod cameras;
pub mod theme_catalog;

use std::{env, fs, io, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "Follow system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    const fn raw(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preferences {
    pub appearance: Appearance,
    pub light_theme_index: usize,
    pub dark_theme_index: usize,
    pub compact: bool,
    pub run_at_startup: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            light_theme_index: theme_catalog::default_index(false),
            dark_theme_index: theme_catalog::default_index(true),
            compact: false,
            run_at_startup: false,
        }
    }
}

impl Preferences {
    pub fn theme_index(self, dark: bool) -> usize {
        if dark {
            self.dark_theme_index
        } else {
            self.light_theme_index
        }
    }

    pub fn set_theme_index(&mut self, dark: bool, index: usize) {
        if dark {
            self.dark_theme_index = index;
        } else {
            self.light_theme_index = index;
        }
    }

    pub fn theme_id(self, dark: bool) -> &'static str {
        theme_catalog::get(self.theme_index(dark))
            .map(|theme| theme.id.as_str())
            .unwrap_or(if dark {
                "Default Dark"
            } else {
                "Default Light"
            })
    }

    fn decode(input: &str) -> Self {
        let mut prefs = Self::default();

        let valid_version = input
            .lines()
            .any(|line| matches!(line, "version=1" | "version=2" | "version=3"));

        if !valid_version {
            return prefs;
        }

        let mut light_id = None;
        let mut dark_id = None;
        let mut previous_id = None;
        let mut legacy_family = None;

        for line in input.lines() {
            if let Some(id) = line.strip_prefix("theme_id_light=") {
                light_id = Some(id.trim().to_owned());
                continue;
            }

            if let Some(id) = line.strip_prefix("theme_id_dark=") {
                dark_id = Some(id.trim().to_owned());
                continue;
            }

            if let Some(id) = line.strip_prefix("theme_id=") {
                previous_id = Some(id.trim().to_owned());
                continue;
            }

            if let Some(family) = line.strip_prefix("theme=") {
                legacy_family = Some(family.trim().to_owned());
                continue;
            }

            match line {
                "appearance=light" => {
                    prefs.appearance = Appearance::Light;
                }
                "appearance=dark" => {
                    prefs.appearance = Appearance::Dark;
                }
                "appearance=system" => {
                    prefs.appearance = Appearance::System;
                }
                "density=compact" => {
                    prefs.compact = true;
                }
                "density=comfortable" => {
                    prefs.compact = false;
                }
                "startup=true" => {
                    prefs.run_at_startup = true;
                }
                "startup=false" => {
                    prefs.run_at_startup = false;
                }
                _ => {}
            }
        }

        if let Some(family) = legacy_family {
            prefs.light_theme_index = theme_catalog::legacy_family_index(&family, false);

            prefs.dark_theme_index = theme_catalog::legacy_family_index(&family, true);
        }

        if let Some(id) = previous_id
            && let Some(index) = theme_catalog::index_of(&id)
            && let Some(theme) = theme_catalog::get(index)
        {
            prefs.set_theme_index(theme.dark, index);
        }

        if let Some(id) = light_id
            && let Some(index) = theme_catalog::index_of(&id)
            && theme_catalog::get(index).is_some_and(|theme| !theme.dark)
        {
            prefs.light_theme_index = index;
        }

        if let Some(id) = dark_id
            && let Some(index) = theme_catalog::index_of(&id)
            && theme_catalog::get(index).is_some_and(|theme| theme.dark)
        {
            prefs.dark_theme_index = index;
        }

        prefs
    }

    fn encode(self) -> String {
        format!(
            "version=3\n\
             appearance={}\n\
             theme_id_light={}\n\
             theme_id_dark={}\n\
             density={}\n\
             startup={}\n",
            self.appearance.raw(),
            self.theme_id(false),
            self.theme_id(true),
            if self.compact {
                "compact"
            } else {
                "comfortable"
            },
            self.run_at_startup,
        )
    }

    pub fn load() -> Self {
        pref_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|input| Self::decode(&input))
            .unwrap_or_default()
    }

    pub fn save(self) -> io::Result<()> {
        let dest = pref_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "No user configuration directory")
        })?;

        let parent = dest
            .parent()
            .ok_or_else(|| io::Error::other("Invalid preferences directory"))?;

        fs::create_dir_all(parent)?;

        let temp = dest.with_extension("tmp");
        fs::write(&temp, self.encode())?;

        if let Err(error) = fs::rename(&temp, &dest) {
            if dest.exists() {
                fs::remove_file(&dest)?;
                fs::rename(&temp, &dest)
            } else {
                Err(error)
            }
        } else {
            Ok(())
        }
    }
}

fn pref_path() -> Option<PathBuf> {
    if cfg!(windows) {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("vermouth-frontend").join("preferences.conf"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|path| path.join("vermouth-frontend").join("preferences.conf"))
    }
}

pub fn set_run_at_startup(enable: bool) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

        let mut cmd = Command::new("reg.exe");

        if enable {
            let exe = env::current_exe()?;
            let exe_text = exe.to_string_lossy();

            if exe_text.contains('"') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unsupported executable path",
                ));
            }

            cmd.args(["add", KEY, "/v", "Vermouth", "/t", "REG_SZ", "/d"])
                .arg(format!("\"{exe_text}\""))
                .arg("/f");
        } else {
            cmd.args(["delete", KEY, "/v", "Vermouth", "/f"]);
        }

        cmd.creation_flags(0x08000000);

        let output = cmd.output()?;

        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(
                "Could not update Windows startup registration",
            ))
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = enable;

        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Run at startup is not supported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip() {
        let mut prefs = Preferences {
            appearance: Appearance::Dark,
            compact: true,
            run_at_startup: true,
            ..Preferences::default()
        };

        let index = theme_catalog::index_of("Catppuccin Macchiato").expect("Macchiato must exist");

        prefs.set_theme_index(true, index);

        assert_eq!(Preferences::decode(&prefs.encode()), prefs);
    }

    #[test]
    fn migrates_original_family_preferences() {
        let prefs = Preferences::decode(
            "version=1\n\
             appearance=dark\n\
             theme=catppuccin\n\
             density=comfortable\n\
             startup=false\n",
        );

        assert_eq!(prefs.theme_id(false), "Catppuccin Latte");

        assert_eq!(prefs.theme_id(true), "Catppuccin Mocha");
    }

    #[test]
    fn migrates_previous_exact_theme_preferences() {
        let prefs = Preferences::decode(
            "version=2\n\
             appearance=dark\n\
             theme_id=Catppuccin Macchiato\n",
        );

        assert_eq!(prefs.theme_id(true), "Catppuccin Macchiato");
    }

    #[test]
    fn light_and_dark_selections_are_independent() {
        let mut prefs = Preferences::default();

        let macchiato = theme_catalog::index_of("Catppuccin Macchiato").unwrap();

        let latte = theme_catalog::index_of("Catppuccin Latte").unwrap();

        prefs.set_theme_index(true, macchiato);
        prefs.set_theme_index(false, latte);

        assert_eq!(prefs.theme_id(true), "Catppuccin Macchiato");

        assert_eq!(prefs.theme_id(false), "Catppuccin Latte");
    }

    #[test]
    fn invalid_version_uses_defaults() {
        assert_eq!(
            Preferences::decode("version=999\nappearance=dark\n"),
            Preferences::default()
        );
    }
}
