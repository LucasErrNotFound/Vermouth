pub mod cameras;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeFamily {
    #[default]
    Default,
    Adventure,
    Alduin,
    Ascinema,
    Aurora,
    Ayu,
    Catppuccin,
    Everforest,
    Fahrenheit,
    Flexoki,
    Gruvbox,
    Harper,
    Hybrid,
    Jellybeans,
    Kibble,
    MacosClassic,
    Mellifluous,
    Molokai,
    Solarized,
    Spaceduck,
    Tokyonight,
    Twilight,
    Graphite,
}

impl ThemeFamily {
    pub const ALL: [Self; 23] = [
        Self::Default,
        Self::Adventure,
        Self::Alduin,
        Self::Ascinema,
        Self::Aurora,
        Self::Ayu,
        Self::Catppuccin,
        Self::Everforest,
        Self::Fahrenheit,
        Self::Flexoki,
        Self::Gruvbox,
        Self::Harper,
        Self::Hybrid,
        Self::Jellybeans,
        Self::Kibble,
        Self::MacosClassic,
        Self::Mellifluous,
        Self::Molokai,
        Self::Solarized,
        Self::Spaceduck,
        Self::Tokyonight,
        Self::Twilight,
        Self::Graphite,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Adventure => "Adventure",
            Self::Alduin => "Alduin",
            Self::Ascinema => "Ascinema",
            Self::Aurora => "Aurora",
            Self::Ayu => "Ayu",
            Self::Catppuccin => "Catppuccin",
            Self::Everforest => "Everforest",
            Self::Fahrenheit => "Fahrenheit",
            Self::Flexoki => "Flexoki",
            Self::Gruvbox => "Gruvbox",
            Self::Harper => "Harper",
            Self::Hybrid => "Hybrid",
            Self::Jellybeans => "Jellybeans",
            Self::Kibble => "Kibble",
            Self::MacosClassic => "macOS Classic",
            Self::Mellifluous => "Mellifluous",
            Self::Molokai => "Molokai",
            Self::Solarized => "Solarized",
            Self::Spaceduck => "Spaceduck",
            Self::Tokyonight => "Tokyo Night",
            Self::Twilight => "Twilight",
            Self::Graphite => "Graphite",
        }
    }

    const fn raw(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Adventure => "adventure",
            Self::Alduin => "alduin",
            Self::Ascinema => "ascinema",
            Self::Aurora => "aurora",
            Self::Ayu => "ayu",
            Self::Catppuccin => "catppuccin",
            Self::Everforest => "everforest",
            Self::Fahrenheit => "fahrenheit",
            Self::Flexoki => "flexoki",
            Self::Gruvbox => "gruvbox",
            Self::Harper => "harper",
            Self::Hybrid => "hybrid",
            Self::Jellybeans => "jellybeans",
            Self::Kibble => "kibble",
            Self::MacosClassic => "macos-classic",
            Self::Mellifluous => "mellifluous",
            Self::Molokai => "molokai",
            Self::Solarized => "solarized",
            Self::Spaceduck => "spaceduck",
            Self::Tokyonight => "tokyonight",
            Self::Twilight => "twilight",
            Self::Graphite => "graphite",
        }
    }

    fn from_raw(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.raw() == value)
    }

    pub fn variant_name(self, dark: bool) -> String {
        format!("{} {}", self.name(), if dark { "Dark" } else { "Light" })
    }

    pub const fn has_local_palette(self) -> bool {
        matches!(
            self,
            Self::Default | Self::Gruvbox | Self::Catppuccin | Self::Ayu | Self::Graphite
        )
    }

    pub fn matches_registry_name(self, name: &str, dark: bool) -> bool {
        let key = name
            .to_ascii_lowercase()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>();

        let prefix = self
            .raw()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>();

        if self == Self::Catppuccin {
            return if dark {
                key == "catppuccinmocha" || key == "catppuccindark"
            } else {
                key == "catppuccinlatte" || key == "catppuccinlight"
            };
        }

        key == format!("{}{}", prefix, if dark { "dark" } else { "light" })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Preferences {
    pub appearance: Appearance,
    pub family: ThemeFamily,
    pub compact: bool,
    pub run_at_startup: bool,
}

impl Preferences {
    fn decode(input: &str) -> Self {
        let mut prefs = Self::default();

        if !input.lines().any(|line| line == "version=1") {
            return prefs;
        }

        for line in input.lines() {
            if let Some(value) = line.strip_prefix("theme=")
                && let Some(family) = ThemeFamily::from_raw(value)
            {
                prefs.family = family;
            }

            match line {
                "appearance=light" => prefs.appearance = Appearance::Light,
                "appearance=dark" => prefs.appearance = Appearance::Dark,
                "appearance=system" => prefs.appearance = Appearance::System,
                "density=compact" => prefs.compact = true,
                "density=comfortable" => prefs.compact = false,
                "startup=true" => prefs.run_at_startup = true,
                "startup=false" => prefs.run_at_startup = false,
                _ => (),
            }
        }

        prefs
    }

    fn encode(self) -> String {
        format!(
            "version=1\nappearance={}\ntheme={}\ndensity={}\nstartup={}\n",
            self.appearance.raw(),
            self.family.raw(),
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

        let parent = dest.parent().expect("preferences file has a directory");

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
            .map(|path| path.join("Vermouth").join("preferences.conf"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|path| path.join("Vermouth").join("preferences.conf"))
    }
}

pub fn set_run_at_startup(enable: bool) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
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
            "Run at startup is not yet supported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_and_legacy_format() {
        let prefs = Preferences {
            appearance: Appearance::Dark,
            family: ThemeFamily::Gruvbox,
            compact: true,
            run_at_startup: true,
        };

        assert_eq!(Preferences::decode(&prefs.encode()), prefs);

        assert_eq!(
            Preferences::decode("version=1\nappearance=dark\n").family,
            ThemeFamily::Default
        );

        assert_eq!(
            Preferences::decode("version=999\nappearance=dark"),
            Preferences::default()
        );
    }
}
