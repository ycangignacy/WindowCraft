// Made by ycangignacy
//! Design tokens. Surfaces and ink come from one palette per appearance.

use iced::{Color, Font};

pub const SANS: Font = Font::with_name("Segoe UI Variable Text");
pub const SANS_STRONG: Font =
    Font { weight: iced::font::Weight::Semibold, ..Font::with_name("Segoe UI Variable Text") };
pub const MONO: Font = Font::with_name("Cascadia Mono");

pub const SIDEBAR_WIDTH: f32 = 220.0;
pub const INSPECTOR_WIDTH: f32 = 340.0;
pub const ROW_HEIGHT: f32 = 42.0;
pub const NAV_ROW_HEIGHT: f32 = 34.0;
pub const GROUP_RADIUS: f32 = 9.0;
pub const CONTROL_RADIUS: f32 = 6.0;
pub const ROW_INSET: f32 = 14.0;

const fn hex(value: u32) -> Color {
    let [_, r, g, b] = value.to_be_bytes();
    Color::from_rgb8(r, g, b)
}

const fn white(alpha: f32) -> Color {
    Color { r: 1.0, g: 1.0, b: 1.0, a: alpha }
}

const fn black(alpha: f32) -> Color {
    Color { r: 0.0, g: 0.0, b: 0.0, a: alpha }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub base: Color,
    pub sidebar: Color,
    pub surface: Color,
    pub elevated: Color,
    pub selected: Color,
    pub hairline: Color,
    pub border: Color,
    pub specular: Color,
    pub ink: Color,
    pub ink_secondary: Color,
    pub ink_tertiary: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub green: Color,
    pub amber: Color,
    pub red: Color,
    pub dark: bool,
}

impl Palette {
    /// Obsidian and Graphite.
    pub const DARK: Self = Self {
        base: hex(0x0B0C0E),
        sidebar: Color { r: 0.043, g: 0.047, b: 0.055, a: 0.72 },
        surface: hex(0x121417),
        elevated: hex(0x181B20),
        selected: hex(0x1F232A),
        hairline: white(0.05),
        border: white(0.07),
        specular: white(0.10),
        ink: hex(0xE8EAED),
        ink_secondary: hex(0x9BA1AA),
        ink_tertiary: hex(0x6B7280),
        accent: hex(0x0A84FF),
        on_accent: hex(0xFFFFFF),
        green: hex(0x30D158),
        amber: hex(0xFF9F0A),
        red: hex(0xFF453A),
        dark: true,
    };

    /// Warm Stone and Paper.
    pub const LIGHT: Self = Self {
        base: hex(0xF5F5F7),
        sidebar: Color { r: 0.93, g: 0.93, b: 0.94, a: 0.72 },
        surface: hex(0xFFFFFF),
        elevated: hex(0xF0F0F2),
        selected: hex(0xE4E6EB),
        hairline: black(0.06),
        border: black(0.08),
        specular: white(0.70),
        ink: hex(0x1C1C1E),
        ink_secondary: hex(0x5C5F66),
        ink_tertiary: hex(0x8A8D93),
        accent: hex(0x007AFF),
        on_accent: hex(0xFFFFFF),
        green: hex(0x248A3D),
        amber: hex(0xB25000),
        red: hex(0xD70015),
        dark: false,
    };

    /// Solid surfaces and maximum contrast.
    pub const HIGH_CONTRAST: Self = Self {
        base: hex(0x000000),
        sidebar: hex(0x000000),
        surface: hex(0x000000),
        elevated: hex(0x1A1A1A),
        selected: hex(0x333333),
        hairline: white(0.6),
        border: white(1.0),
        specular: white(0.0),
        ink: hex(0xFFFFFF),
        ink_secondary: hex(0xFFFFFF),
        ink_tertiary: hex(0xE0E0E0),
        accent: hex(0x4CC2FF),
        on_accent: hex(0x000000),
        green: hex(0x7CFC00),
        amber: hex(0xFFD400),
        red: hex(0xFF6B6B),
        dark: true,
    };

    /// Opaque sidebar when transparency is off.
    pub fn solid(mut self) -> Self {
        self.sidebar = Color { a: 1.0, ..self.sidebar };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_purple(color: Color) -> bool {
        color.b > 0.5 && color.r > 0.3 && color.g < 0.3
    }

    #[test]
    fn palettes_contain_no_purple() {
        for palette in [Palette::DARK, Palette::LIGHT, Palette::HIGH_CONTRAST] {
            for color in [palette.base, palette.accent, palette.surface, palette.selected] {
                assert!(!is_purple(color));
            }
        }
    }

    #[test]
    fn specified_tokens_are_exact() {
        assert_eq!(Palette::DARK.base, Color::from_rgb8(0x0B, 0x0C, 0x0E));
        assert_eq!(Palette::DARK.surface, Color::from_rgb8(0x12, 0x14, 0x17));
        assert_eq!(Palette::DARK.elevated, Color::from_rgb8(0x18, 0x1B, 0x20));
        assert_eq!(Palette::DARK.selected, Color::from_rgb8(0x1F, 0x23, 0x2A));
        assert_eq!(Palette::DARK.accent, Color::from_rgb8(0x0A, 0x84, 0xFF));
        assert_eq!(Palette::LIGHT.accent, Color::from_rgb8(0x00, 0x7A, 0xFF));
        assert_eq!(Palette::DARK.green, Color::from_rgb8(0x30, 0xD1, 0x58));
    }
}
