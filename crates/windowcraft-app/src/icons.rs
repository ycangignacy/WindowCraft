// Made by WindowCraft Dev Team
//! Monochrome stroke icons on a 24 unit grid with a 1.5 stroke, drawn in the
//! style of Lucide. See NOTICE.md and assets/LUCIDE-LICENSE.txt.

use iced::widget::{Svg, svg};
use iced::{Color, Length};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Apps,
    Tweaks,
    Wrench,
    Refresh,
    Disc,
    Clock,
    Gauge,
    Search,
    Eye,
    Close,
    Check,
    Alert,
    Download,
    Trash,
    Undo,
    Play,
    Stop,
    Sun,
    Moon,
    Chevron,
    Folder,
    Terminal,
    Gear,
    Layers,
}

impl Icon {
    fn body(self) -> &'static str {
        match self {
            Self::Apps => r#"<rect x="3.75" y="3.75" width="7" height="7" rx="1.5"/><rect x="13.25" y="3.75" width="7" height="7" rx="1.5"/><rect x="3.75" y="13.25" width="7" height="7" rx="1.5"/><rect x="13.25" y="13.25" width="7" height="7" rx="1.5"/>"#,
            Self::Tweaks => r#"<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2.25"/><circle cx="9" cy="17" r="2.25"/>"#,
            Self::Wrench => r#"<path d="M14.7 6.3a4 4 0 0 1 5.1-1l-3 3 1.9 1.9 3-3a4 4 0 0 1-5.4 5.4L8.6 20.3a2 2 0 0 1-2.9-2.9l7.7-7.7a4 4 0 0 1 1.3-3.4z"/>"#,
            Self::Refresh => r#"<path d="M20 11a8 8 0 0 0-14.3-4L4 9"/><path d="M4 4v5h5"/><path d="M4 13a8 8 0 0 0 14.3 4L20 15"/><path d="M20 20v-5h-5"/>"#,
            Self::Disc => r#"<circle cx="12" cy="12" r="8.25"/><circle cx="12" cy="12" r="2.25"/>"#,
            Self::Clock => r#"<circle cx="12" cy="12" r="8.25"/><path d="M12 7.5V12l3 2"/>"#,
            Self::Gauge => r#"<path d="M4.5 18a8.25 8.25 0 1 1 15 0"/><path d="M12 14l4-4.5"/><circle cx="12" cy="14" r="1.25"/>"#,
            Self::Search => r#"<circle cx="10.75" cy="10.75" r="6.5"/><path d="M15.5 15.5 20 20"/>"#,
            Self::Eye => r#"<path d="M2.75 12S6 5.75 12 5.75 21.25 12 21.25 12 18 18.25 12 18.25 2.75 12 2.75 12z"/><circle cx="12" cy="12" r="2.75"/>"#,
            Self::Close => r#"<path d="M6 6l12 12M18 6 6 18"/>"#,
            Self::Check => r#"<path d="M5 12.5l4.5 4.5L19 7.5"/>"#,
            Self::Alert => r#"<path d="M12 4 2.75 19.5h18.5L12 4z"/><path d="M12 10v4.5M12 17v.25"/>"#,
            Self::Download => r#"<path d="M12 4v11M7.5 11 12 15.5 16.5 11"/><path d="M4.5 19.5h15"/>"#,
            Self::Trash => r#"<path d="M4.5 7h15M9.5 7V4.75h5V7M6.5 7l1 12.25h9L17.5 7"/>"#,
            Self::Undo => r#"<path d="M9 5 4.5 9.5 9 14"/><path d="M4.5 9.5H14a5.5 5.5 0 0 1 0 11H9"/>"#,
            Self::Play => r#"<path d="M7.5 4.75v14.5L19 12 7.5 4.75z"/>"#,
            Self::Stop => r#"<rect x="6" y="6" width="12" height="12" rx="2"/>"#,
            Self::Sun => r#"<circle cx="12" cy="12" r="3.75"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4"/>"#,
            Self::Moon => r#"<path d="M20 14.5A8.25 8.25 0 1 1 9.5 4a6.75 6.75 0 0 0 10.5 10.5z"/>"#,
            Self::Chevron => r#"<path d="M9.5 6l6 6-6 6"/>"#,
            Self::Folder => r#"<path d="M3.75 7.5A2 2 0 0 1 5.75 5.5h3.5l2 2.25h7a2 2 0 0 1 2 2v7.75a2 2 0 0 1-2 2H5.75a2 2 0 0 1-2-2V7.5z"/>"#,
            Self::Terminal => r#"<rect x="3.25" y="4.75" width="17.5" height="14.5" rx="2"/><path d="M7 9.5l3 2.5-3 2.5M12.5 15h4.5"/>"#,
            Self::Layers => r#"<path d="M12 3.75 3.75 8 12 12.25 20.25 8 12 3.75z"/><path d="M3.75 12 12 16.25 20.25 12"/><path d="M3.75 16 12 20.25 20.25 16"/>"#,
            Self::Gear => r#"<path d="M10.33 5.3 10.73 2.99h2.54l.4 2.31a6.9 6.9 0 0 1 1.88.79l1.93-1.36 1.79 1.79-1.36 1.93a6.9 6.9 0 0 1 .79 1.88l2.31.4v2.54l-2.31.4a6.9 6.9 0 0 1-.79 1.88l1.36 1.93-1.79 1.79-1.93-1.36a6.9 6.9 0 0 1-1.88.79l-.4 2.31h-2.54l-.4-2.31a6.9 6.9 0 0 1-1.88-.79l-1.93 1.36-1.79-1.79 1.36-1.93a6.9 6.9 0 0 1-.79-1.88l-2.31-.4v-2.54l2.31-.4a6.9 6.9 0 0 1 .79-1.88L4.73 6.52l1.79-1.79 1.93 1.36a6.9 6.9 0 0 1 1.88-.79z"/><circle cx="12" cy="12" r="2.75"/>"#,
        }
    }

    fn document(self) -> Vec<u8> {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
        .into_bytes()
    }

    pub fn view<'a>(self, size: f32, color: Color) -> Svg<'a> {
        svg(svg::Handle::from_memory(self.document()))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .style(move |_, _| svg::Style { color: Some(color) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ICONS: [Icon; 24] = [
        Icon::Apps,
        Icon::Tweaks,
        Icon::Wrench,
        Icon::Refresh,
        Icon::Disc,
        Icon::Clock,
        Icon::Gauge,
        Icon::Search,
        Icon::Eye,
        Icon::Close,
        Icon::Check,
        Icon::Alert,
        Icon::Download,
        Icon::Trash,
        Icon::Undo,
        Icon::Play,
        Icon::Stop,
        Icon::Sun,
        Icon::Moon,
        Icon::Chevron,
        Icon::Folder,
        Icon::Terminal,
        Icon::Gear,
        Icon::Layers,
    ];

    #[test]
    fn every_icon_uses_the_same_stroke() {
        for icon in ALL_ICONS {
            let doc = String::from_utf8(icon.document()).unwrap();
            assert!(doc.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24""#));
            assert!(doc.ends_with("</svg>"));
            assert!(doc.contains(r#"stroke-width="1.5""#));
            assert!(doc.contains(r#"fill="none""#));
            assert!(!icon.body().is_empty());
            assert_eq!(doc.matches('<').count(), doc.matches('>').count());
            let _ = icon.view(16.0, Color::WHITE);
        }
    }

    #[test]
    fn every_icon_variant_has_a_distinct_svg_body() {
        for (i, first) in ALL_ICONS.iter().enumerate() {
            for second in &ALL_ICONS[i + 1..] {
                assert_ne!(first.body(), second.body(), "{first:?} and {second:?} share the same SVG body");
            }
        }
    }
}
