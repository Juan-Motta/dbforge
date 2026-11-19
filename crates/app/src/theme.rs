//! Light and dark, as the user chose them: one of the two, or whichever the
//! OS shows ("system"). The colours are the Liquid Glass look's: a near-solid
//! wall, translucent panels with a light rim, and the terracotta accent.

use dbforge_core::connections::tags::TagColor;
use gpui::{App, BorrowAppContext, Global, Hsla, WindowAppearance, rgba};

use crate::preferences::{Preferences, keys};

/// What the user chose in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

impl ThemePreference {
    /// The saved choice; dark when nothing (or something unknown) was saved, as before.
    pub fn parse(saved: Option<&str>) -> Self {
        match saved {
            Some("light") => Self::Light,
            Some("system") => Self::System,
            _ => Self::Dark,
        }
    }

    /// As it is saved under [`keys::THEME`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// The preference saved in `preferences`.
    pub fn saved(preferences: &Preferences) -> Self {
        Self::parse(preferences.get(keys::THEME))
    }

    /// Saves it as the user's choice; the window, observing the
    /// preferences, shows it at once.
    pub fn save(self, cx: &mut App) {
        cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::THEME, Some(self.as_str().into())));
    }
}

/// Light or dark, as shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    /// The one `preference` shows while the OS shows `system`.
    pub fn resolve(preference: ThemePreference, system: Appearance) -> Self {
        match preference {
            ThemePreference::System => system,
            ThemePreference::Light => Self::Light,
            ThemePreference::Dark => Self::Dark,
        }
    }
}

impl From<WindowAppearance> for Appearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }
}

/// The monospace font each OS has: for SQL, values and shortcuts.
pub const MONO: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(target_os = "windows") {
    "Consolas"
} else {
    "DejaVu Sans Mono"
};

/// The colours the window is drawn with now, a GPUI global ([`ActiveTheme`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub appearance: Appearance,
    /// The window's background, top to bottom.
    pub wall_top: Hsla,
    pub wall_bottom: Hsla,
    /// Text, from the most to the least prominent.
    pub text: Hsla,
    pub text2: Hsla,
    pub text3: Hsla,
    /// Translucent panels, a raised segment on them, and the editor's card.
    pub glass: Hsla,
    pub glass_hi: Hsla,
    pub card: Hsla,
    /// The light edge of a panel.
    pub rim: Hsla,
    pub line: Hsla,
    pub sel: Hsla,
    /// Behind a dialog.
    pub scrim: Hsla,
    pub accent: Hsla,
    pub accent_hi: Hsla,
    pub accent_text: Hsla,
    pub kw: Hsla,
    pub str: Hsla,
    pub num: Hsla,
    pub func: Hsla,
    pub com: Hsla,
    pub ok: Hsla,
    pub err: Hsla,
    pub warn: Hsla,
}

impl Global for Theme {}

/// A colour from `0xRRGGBBAA`.
pub fn color(hex: u32) -> Hsla {
    rgba(hex).into()
}

impl Theme {
    pub fn new(appearance: Appearance) -> Self {
        let dark = appearance == Appearance::Dark;
        let pick = |d: u32, l: u32| color(if dark { d } else { l });
        Self {
            appearance,
            wall_top: pick(0x17161BFF, 0xECEBEFFF),
            wall_bottom: pick(0x0F0E12FF, 0xE2E1E6FF),
            text: pick(0xF4F2F7FF, 0x1C1A22FF),
            text2: pick(0xC9C5D3FF, 0x4A4656FF),
            text3: pick(0x9A95A8FF, 0x6E6979FF),
            glass: pick(0x222128C7, 0xFFFFFFBD),
            glass_hi: pick(0xFFFFFF1A, 0xFFFFFFE6),
            card: pick(0x16151BE6, 0xFFFFFFEB),
            rim: pick(0xFFFFFF29, 0xFFFFFFB3),
            line: pick(0xFFFFFF14, 0x1C1A2214),
            sel: pick(0xFFFFFF1F, 0x1C1A2212),
            scrim: pick(0x0000008C, 0x1C1A2240),
            accent: color(0xD45325FF),
            accent_hi: color(0xE8703EFF),
            accent_text: pick(0xF2A07AFF, 0xB23E14FF),
            kw: pick(0xF2A07AFF, 0xB23E14FF),
            str: pick(0x9FD8A8FF, 0x2E7D44FF),
            num: pick(0xB9A6FFFF, 0x5B3FC4FF),
            func: pick(0x7FC6E8FF, 0x1F6C93FF),
            com: pick(0x8D879BFF, 0x77717FFF),
            ok: pick(0x7FD69AFF, 0x2E7D44FF),
            err: pick(0xFF8A80FF, 0xC2331FFF),
            warn: color(0xE0A33AFF),
        }
    }

    /// A tag's colour in this theme: the former web UI's palette, lighter on dark.
    pub fn tag(&self, tag: TagColor) -> Hsla {
        let dark = self.appearance == Appearance::Dark;
        let (d, l) = match tag {
            TagColor::Red => (0xF06A6AFF, 0xC23030FF),
            TagColor::Orange => (0xF0905AFF, 0xB4531CFF),
            TagColor::Amber => (0xE8B44CFF, 0x8F6400FF),
            TagColor::Green => (0x4CC38AFF, 0x1E8553FF),
            TagColor::Teal => (0x3CC0B4FF, 0x0F7F78FF),
            TagColor::Blue => (0x6AA9F0FF, 0x2468B8FF),
            TagColor::Violet => (0xA98BF0FF, 0x6B46C1FF),
            TagColor::Pink => (0xEC7FB4FF, 0xB02A6CFF),
            TagColor::Gray => (0x9A9CA3FF, 0x5F6168FF),
        };
        color(if dark { d } else { l })
    }

    /// Sets the theme the saved preference shows while the OS shows `system`;
    /// whether it changed, so the caller redraws only then.
    pub fn sync(system: Appearance, cx: &mut App) -> bool {
        let appearance = Appearance::resolve(ThemePreference::saved(Preferences::global(cx)), system);
        if cx.try_global::<Theme>().is_some_and(|theme| theme.appearance == appearance) {
            return false;
        }
        cx.set_global(Theme::new(appearance));
        true
    }
}

/// The theme in use, from any context.
pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_saved_choice_as_the_web_ui_did() {
        assert_eq!(ThemePreference::parse(Some("light")), ThemePreference::Light);
        assert_eq!(ThemePreference::parse(Some("system")), ThemePreference::System);
        assert_eq!(ThemePreference::parse(Some("dark")), ThemePreference::Dark);
        assert_eq!(ThemePreference::parse(None), ThemePreference::Dark, "dark until one is chosen");
        assert_eq!(ThemePreference::parse(Some("sepia")), ThemePreference::Dark);
        for preference in [ThemePreference::System, ThemePreference::Light, ThemePreference::Dark] {
            assert_eq!(ThemePreference::parse(Some(preference.as_str())), preference);
        }
    }

    #[test]
    fn system_follows_the_os_and_a_choice_ignores_it() {
        use Appearance::{Dark, Light};
        assert_eq!(Appearance::resolve(ThemePreference::System, Dark), Dark);
        assert_eq!(Appearance::resolve(ThemePreference::System, Light), Light);
        assert_eq!(Appearance::resolve(ThemePreference::Light, Dark), Light);
        assert_eq!(Appearance::resolve(ThemePreference::Dark, Light), Dark);
        assert_eq!(Appearance::from(WindowAppearance::VibrantDark), Dark);
        assert_eq!(Appearance::from(WindowAppearance::VibrantLight), Light);
    }

    #[test]
    fn tags_read_in_both_themes_as_the_web_ui_drew_them() {
        assert_eq!(Theme::new(Appearance::Dark).tag(TagColor::Red), color(0xF06A6AFF));
        assert_eq!(Theme::new(Appearance::Light).tag(TagColor::Red), color(0xC23030FF));
    }

    #[test]
    fn the_accent_is_terracotta_in_both() {
        assert_eq!(Theme::new(Appearance::Dark).accent, color(0xD45325FF));
        assert_eq!(Theme::new(Appearance::Light).accent, color(0xD45325FF));
        assert_ne!(Theme::new(Appearance::Dark).text, Theme::new(Appearance::Light).text);
    }
}
