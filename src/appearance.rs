use crate::geometry::Geometry;
use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Paper,
    Ivory,
    Mint,
    Sky,
    Lavender,
    #[default]
    Midnight,
    Charcoal,
    Forest,
    Ocean,
    Plum,
}

impl Theme {
    pub const ALL: [Self; 10] = [
        Self::Paper,
        Self::Ivory,
        Self::Mint,
        Self::Sky,
        Self::Lavender,
        Self::Midnight,
        Self::Charcoal,
        Self::Forest,
        Self::Ocean,
        Self::Plum,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Paper => "페이퍼",
            Self::Ivory => "아이보리",
            Self::Mint => "민트",
            Self::Sky => "스카이",
            Self::Lavender => "라벤더",
            Self::Midnight => "미드나이트",
            Self::Charcoal => "차콜",
            Self::Forest => "포레스트",
            Self::Ocean => "오션",
            Self::Plum => "플럼",
        }
    }

    pub fn dark(self) -> bool {
        matches!(
            self,
            Self::Midnight | Self::Charcoal | Self::Forest | Self::Ocean | Self::Plum
        )
    }

    pub fn palette(self) -> Palette {
        let (background, cell, outside, today, text, muted, accent) = match self {
            Self::Paper => (
                0xf3f4f6, 0xffffff, 0xe8ebef, 0xdbeafe, 0x202936, 0x596579, 0x2563eb,
            ),
            Self::Ivory => (
                0xf5f0e6, 0xfffcf4, 0xeae3d5, 0xf8dfac, 0x403529, 0x74634e, 0x986019,
            ),
            Self::Mint => (
                0xeaf5ef, 0xf8fffb, 0xd9e9e0, 0xbfe9d1, 0x203e31, 0x506d5d, 0x16734a,
            ),
            Self::Sky => (
                0xeaf3fb, 0xf8fcff, 0xd9e6f2, 0xbfdef8, 0x243b53, 0x536d86, 0x1767ad,
            ),
            Self::Lavender => (
                0xf2edfa, 0xfdfaff, 0xe5ddf0, 0xdcccf5, 0x3b2d50, 0x6e5b86, 0x7950b0,
            ),
            Self::Midnight => (
                0x151c2b, 0x222e43, 0x1a2232, 0x254e78, 0xe8efff, 0xa0b0ca, 0x8cc4ff,
            ),
            Self::Charcoal => (
                0x202124, 0x303236, 0x27282c, 0x4b5058, 0xf0f1f3, 0xb3b8c2, 0xc6cbd4,
            ),
            Self::Forest => (
                0x14261f, 0x21392e, 0x192e25, 0x2b6148, 0xe1f5e9, 0xa0c1ae, 0x80dca6,
            ),
            Self::Ocean => (
                0x102831, 0x1b3b47, 0x15313b, 0x205f72, 0xe0f5fb, 0x9fc0cc, 0x76d4ed,
            ),
            Self::Plum => (
                0x291d30, 0x3b2b46, 0x302236, 0x634170, 0xf6e9fc, 0xc7a8d1, 0xdea0f2,
            ),
        };
        Palette {
            background: rgb(background),
            cell: rgb(cell),
            outside: rgb(outside),
            today: rgb(today),
            text: rgb(text),
            muted: rgb(muted),
            accent: rgb(accent),
        }
    }

    pub fn apply(self, ctx: &egui::Context) {
        let p = self.palette();
        let mut visuals = if self.dark() {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.override_text_color = Some(p.text);
        visuals.panel_fill = p.background;
        visuals.window_fill = p.background;
        visuals.extreme_bg_color = p.outside;
        visuals.faint_bg_color = p.cell;
        visuals.widgets.noninteractive.bg_fill = p.cell;
        visuals.widgets.inactive.bg_fill = p.cell;
        visuals.widgets.hovered.bg_fill = p.today;
        visuals.widgets.active.bg_fill = p.today;
        visuals.selection.bg_fill = p.today;
        visuals.selection.stroke.color = p.text;
        visuals.hyperlink_color = p.accent;
        ctx.set_visuals(visuals);
    }
}

pub struct Palette {
    pub background: Color32,
    pub cell: Color32,
    pub outside: Color32,
    pub today: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
}
fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    /// 0 = opaque. Keep at least 20% visibility so controls remain recoverable.
    pub transparency: u8,
    pub geometry: Option<Geometry>,
}

impl Appearance {
    pub fn opacity(&self) -> u8 {
        ((100 - self.transparency.min(80)) as u16 * 255 / 100) as u8
    }

    pub fn load() -> (Self, Option<String>) {
        let result = (|| -> anyhow::Result<Self> {
            let path = settings_path()?;
            if !path.exists() {
                return Ok(Self::default());
            }
            let mut settings: Self = serde_json::from_slice(&std::fs::read(path)?)?;
            settings.transparency = settings.transparency.min(80);
            settings.geometry = settings.geometry.filter(|g| g.width > 0 && g.height > 0);
            Ok(settings)
        })();
        match result {
            Ok(settings) => (settings, None),
            Err(error) => (
                Self::default(),
                Some(format!("설정을 불러오지 못했습니다: {error}")),
            ),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = settings_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    }
}

fn settings_path() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".config")))
        .ok_or_else(|| anyhow::anyhow!("사용자 설정 폴더를 찾을 수 없습니다"))?;
    Ok(base.join("DeskCalendar").join("appearance.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn five_light_and_five_dark_themes() {
        assert_eq!(Theme::ALL.iter().filter(|t| t.dark()).count(), 5);
        assert_eq!(Theme::ALL.iter().filter(|t| !t.dark()).count(), 5);
        for theme in Theme::ALL {
            let settings = Appearance {
                theme,
                transparency: 45,
                ..Default::default()
            };
            let json = serde_json::to_string(&settings).unwrap();
            let restored: Appearance = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.theme, theme);
            assert_eq!(restored.transparency, 45);
            assert_ne!(theme.palette().text, theme.palette().cell);
        }
    }
    #[test]
    fn text_has_readable_contrast_in_every_theme() {
        fn luminance(color: Color32) -> f64 {
            let linear = |v: u8| {
                let x = f64::from(v) / 255.0;
                if x <= 0.04045 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        }
        for theme in Theme::ALL {
            let palette = theme.palette();
            for background in [palette.cell, palette.today, palette.background] {
                let text = luminance(palette.text);
                let background = luminance(background);
                let ratio = (text.max(background) + 0.05) / (text.min(background) + 0.05);
                assert!(ratio >= 4.5, "{} contrast: {ratio}", theme.name());
            }
        }
    }

    #[test]
    fn settings_survive_restart_and_corrupt_file_recovers() {
        // Other tests do not read environment variables or settings files.
        struct Restore(Option<std::ffi::OsString>, PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(value) = &self.0 {
                    std::env::set_var("LOCALAPPDATA", value);
                } else {
                    std::env::remove_var("LOCALAPPDATA");
                }
                let _ = std::fs::remove_dir_all(&self.1);
            }
        }
        let directory = std::env::temp_dir().join(format!(
            "desk-calendar-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _restore = Restore(std::env::var_os("LOCALAPPDATA"), directory.clone());
        std::env::set_var("LOCALAPPDATA", &directory);
        assert!(Appearance::load().1.is_none());
        let settings = Appearance {
            theme: Theme::Mint,
            transparency: 55,
            geometry: Some(Geometry {
                x: -1200,
                y: 90,
                width: 900,
                height: 650,
            }),
        };
        settings.save().unwrap();
        let (loaded, error) = Appearance::load();
        assert!(error.is_none());
        assert_eq!(loaded.theme, Theme::Mint);
        assert_eq!(loaded.transparency, 55);
        assert_eq!(loaded.geometry, settings.geometry);
        // Replacing an existing settings file must also work.
        Appearance {
            theme: Theme::Ocean,
            transparency: 255,
            ..Default::default()
        }
        .save()
        .unwrap();
        assert_eq!(Appearance::load().0.transparency, 80);
        std::fs::write(settings_path().unwrap(), b"broken json").unwrap();
        let (loaded, error) = Appearance::load();
        assert!(error.is_some());
        assert_eq!(loaded.theme, Theme::Midnight);
    }

    #[test]
    fn transparency_never_hides_controls_completely() {
        for value in 0..=255 {
            let settings = Appearance {
                transparency: value,
                ..Default::default()
            };
            assert!((51..=255).contains(&settings.opacity()));
        }
        assert_eq!(Appearance::default().opacity(), 255);
        assert_eq!(
            Appearance {
                transparency: 80,
                ..Default::default()
            }
            .opacity(),
            51
        );
        let missing_fields: Appearance = serde_json::from_str("{}").unwrap();
        assert_eq!(missing_fields.theme, Theme::Midnight);
        assert!(missing_fields.geometry.is_none());
    }
}
