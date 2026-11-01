use std::sync::LazyLock;

use gpui_kit::App;
use gpui_kit::component::ThemeRegistry;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct ThemeDescriptor {
    pub id: String,
    pub display_name: String,
    pub family: String,
    pub dark: bool,
}

#[derive(Debug, Deserialize)]
struct ThemeSet {
    name: String,
    themes: Vec<ThemeEntry>,
}

#[derive(Debug, Deserialize)]
struct ThemeEntry {
    name: String,
    mode: String,
}

const BUNDLED_THEMES: [(&str, &str); 21] = [
    (
        "adventure.json",
        include_str!("../../assets/themes/adventure.json"),
    ),
    (
        "alduin.json",
        include_str!("../../assets/themes/alduin.json"),
    ),
    (
        "asciinema.json",
        include_str!("../../assets/themes/asciinema.json"),
    ),
    (
        "aurora.json",
        include_str!("../../assets/themes/aurora.json"),
    ),
    ("ayu.json", include_str!("../../assets/themes/ayu.json")),
    (
        "catppuccin.json",
        include_str!("../../assets/themes/catppuccin.json"),
    ),
    (
        "everforest.json",
        include_str!("../../assets/themes/everforest.json"),
    ),
    (
        "fahrenheit.json",
        include_str!("../../assets/themes/fahrenheit.json"),
    ),
    (
        "flexoki.json",
        include_str!("../../assets/themes/flexoki.json"),
    ),
    (
        "gruvbox.json",
        include_str!("../../assets/themes/gruvbox.json"),
    ),
    (
        "harper.json",
        include_str!("../../assets/themes/harper.json"),
    ),
    (
        "hybrid.json",
        include_str!("../../assets/themes/hybrid.json"),
    ),
    (
        "jellybeans.json",
        include_str!("../../assets/themes/jellybeans.json"),
    ),
    (
        "kibble.json",
        include_str!("../../assets/themes/kibble.json"),
    ),
    (
        "macos-classic.json",
        include_str!("../../assets/themes/macos-classic.json"),
    ),
    (
        "mellifluous.json",
        include_str!("../../assets/themes/mellifluous.json"),
    ),
    (
        "molokai.json",
        include_str!("../../assets/themes/molokai.json"),
    ),
    (
        "solarized.json",
        include_str!("../../assets/themes/solarized.json"),
    ),
    (
        "spaceduck.json",
        include_str!("../../assets/themes/spaceduck.json"),
    ),
    (
        "tokyonight.json",
        include_str!("../../assets/themes/tokyonight.json"),
    ),
    (
        "twilight.json",
        include_str!("../../assets/themes/twilight.json"),
    ),
];

fn has_mode_suffix(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();

    lower.ends_with(" light") || lower.ends_with(" dark")
}

fn display_name(name: &str, dark: bool, single_variant: bool) -> String {
    if !single_variant || has_mode_suffix(name) {
        return name.to_owned();
    }

    format!("{} {}", name, if dark { "Dark" } else { "Light" })
}

static CATALOG: LazyLock<Vec<ThemeDescriptor>> = LazyLock::new(|| {
    let mut catalog = vec![
        ThemeDescriptor {
            id: "Default Light".into(),
            display_name: "Default Light".into(),
            family: "Default".into(),
            dark: false,
        },
        ThemeDescriptor {
            id: "Default Dark".into(),
            display_name: "Default Dark".into(),
            family: "Default".into(),
            dark: true,
        },
    ];

    for (filename, content) in BUNDLED_THEMES {
        let set = match serde_json::from_str::<ThemeSet>(content) {
            Ok(set) => set,
            Err(error) => {
                eprintln!("Could not parse theme file {filename}: {error}");
                continue;
            }
        };

        let single_variant = set.themes.len() == 1;

        for entry in set.themes {
            let dark = match entry.mode.as_str() {
                "light" => false,
                "dark" => true,
                other => {
                    eprintln!("Unknown theme mode '{other}' for {}", entry.name);
                    continue;
                }
            };

            if catalog.iter().any(|theme| theme.id == entry.name) {
                eprintln!("Duplicate theme name: {}", entry.name);
                continue;
            }

            catalog.push(ThemeDescriptor {
                display_name: display_name(&entry.name, dark, single_variant),
                id: entry.name,
                family: set.name.clone(),
                dark,
            });
        }
    }

    catalog
});

pub fn all() -> &'static [ThemeDescriptor] {
    CATALOG.as_slice()
}

pub fn get(index: usize) -> Option<&'static ThemeDescriptor> {
    all().get(index)
}

pub fn index_of(id: &str) -> Option<usize> {
    all().iter().position(|theme| theme.id == id)
}

pub const fn default_index(dark: bool) -> usize {
    if dark { 1 } else { 0 }
}

pub fn ensure_registered(cx: &mut App) {
    for (filename, content) in BUNDLED_THEMES {
        let set = match serde_json::from_str::<ThemeSet>(content) {
            Ok(set) => set,
            Err(error) => {
                eprintln!("Could not parse {filename}: {error}");
                continue;
            }
        };

        let all_registered = set.themes.iter().all(|entry| {
            ThemeRegistry::global(cx)
                .themes()
                .contains_key(entry.name.as_str())
        });

        if all_registered {
            continue;
        }

        if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(content) {
            eprintln!("Could not register {filename}: {error}");
        }
    }
}

pub fn available(cx: &App, dark: bool) -> Vec<(usize, &'static ThemeDescriptor)> {
    let registry = ThemeRegistry::global(cx);

    let mut entries = all()
        .iter()
        .enumerate()
        .filter(|(_, theme)| {
            theme.dark == dark
                && (theme.family == "Default" || registry.themes().contains_key(theme.id.as_str()))
        })
        .collect::<Vec<_>>();

    entries.sort_by(|(_, left), (_, right)| {
        left.family
            .to_ascii_lowercase()
            .cmp(&right.family.to_ascii_lowercase())
            .then_with(|| {
                left.display_name
                    .to_ascii_lowercase()
                    .cmp(&right.display_name.to_ascii_lowercase())
            })
    });

    entries
}

pub fn resolve(cx: &App, selected: usize, dark: bool) -> usize {
    let entries = available(cx, dark);

    if entries.iter().any(|(index, _)| *index == selected) {
        return selected;
    }

    let Some(previous) = get(selected) else {
        return default_index(dark);
    };

    let preferred = match (previous.family.as_str(), dark) {
        ("Catppuccin", true) => Some("Catppuccin Mocha"),
        ("Catppuccin", false) => Some("Catppuccin Latte"),
        _ => None,
    };

    if let Some(preferred) = preferred
        && let Some((index, _)) = entries.iter().find(|(_, theme)| theme.id == preferred)
    {
        return *index;
    }

    entries
        .iter()
        .find(|(_, theme)| theme.family == previous.family)
        .map(|(index, _)| *index)
        .unwrap_or_else(|| default_index(dark))
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn legacy_family_index(family: &str, dark: bool) -> usize {
    let normalized = normalize(family);

    if normalized == "catppuccin" {
        let id = if dark {
            "Catppuccin Mocha"
        } else {
            "Catppuccin Latte"
        };

        if let Some(index) = index_of(id) {
            return index;
        }
    }

    all()
        .iter()
        .enumerate()
        .find(|(_, theme)| theme.dark == dark && normalize(&theme.family) == normalized)
        .map(|(index, _)| index)
        .unwrap_or_else(|| default_index(dark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_bundled_json_files_parse() {
        assert_eq!(BUNDLED_THEMES.len(), 21);

        for (filename, content) in BUNDLED_THEMES {
            let set: ThemeSet =
                serde_json::from_str(content).unwrap_or_else(|error| panic!("{filename}: {error}"));

            assert!(!set.themes.is_empty(), "{filename} contains no themes");

            for theme in set.themes {
                assert!(
                    matches!(theme.mode.as_str(), "light" | "dark"),
                    "Invalid mode for {} in {filename}",
                    theme.name
                );

                assert!(
                    index_of(&theme.name).is_some(),
                    "Missing catalog entry: {}",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn catalog_has_unique_registry_names() {
        let mut ids = std::collections::HashSet::new();

        for theme in all() {
            assert!(ids.insert(&theme.id), "Duplicate theme: {}", theme.id);
        }
    }

    #[test]
    fn catppuccin_variants_are_independent() {
        for id in [
            "Catppuccin Latte",
            "Catppuccin Frappe",
            "Catppuccin Macchiato",
            "Catppuccin Mocha",
        ] {
            assert!(index_of(id).is_some(), "Missing Catppuccin variant: {id}");
        }
    }

    #[test]
    fn multiple_variants_are_not_collapsed() {
        for id in [
            "Adventure",
            "Adventure Time",
            "Tokyo Night",
            "Tokyo Moon",
            "Tokyo Storm",
        ] {
            assert!(index_of(id).is_some(), "Missing multi-variant theme: {id}");
        }
    }

    #[test]
    fn standalone_themes_keep_appearance_suffixes() {
        for id in [
            "Fahrenheit",
            "Harper",
            "Jellybeans",
            "Kibble",
            "Spaceduck",
            "Twilight",
        ] {
            if let Some(index) = index_of(id) {
                let theme = get(index).unwrap();

                assert!(
                    theme.display_name.ends_with(" Dark"),
                    "Missing Dark suffix: {}",
                    theme.display_name
                );
            }
        }
    }

    #[test]
    fn existing_suffix_is_not_duplicated() {
        assert_eq!(display_name("Ayu Dark", true, true), "Ayu Dark");

        assert_eq!(
            display_name("Everforest Light", false, true),
            "Everforest Light"
        );
    }

    #[test]
    fn multi_variant_name_remains_unchanged() {
        assert_eq!(display_name("Tokyo Moon", true, false), "Tokyo Moon");
    }
}
