/// 16 basic colors - the commonly supported format in terminals.
/// Most (looking at you dialoguer) terminal dialog libraries support
/// this format but define their own types for it.
/// Though `env_logger` and `inquire` we use two separate implementations
/// of basic colors.
/// To support both we add an abstraction that can be converted into either.
#[allow(dead_code)]
pub enum BasicColor {
    Black,       // 0
    DarkRed,     // 1
    DarkGreen,   // 2
    DarkYellow,  // 3
    DarkBlue,    // 4
    DarkMagenta, // 5
    DarkCyan,    // 6
    Grey,        // 7
    DarkGrey,    // 8

    Red,     // 9
    Green,   // 10
    Yellow,  // 11
    Blue,    // 12
    Magenta, // 13
    Cyan,    // 14
    White,   // 15
}

impl BasicColor {
    /// Create crossterm compatible types
    #[allow(dead_code)] // todo: discuss how/where to integrate colors
    pub fn to_crossterm(&self) -> crossterm::style::Color {
        match self {
            BasicColor::Black => crossterm::style::Color::Black,
            BasicColor::DarkRed => crossterm::style::Color::DarkRed,
            BasicColor::DarkGreen => crossterm::style::Color::DarkGreen,
            BasicColor::DarkYellow => crossterm::style::Color::DarkYellow,
            BasicColor::DarkBlue => crossterm::style::Color::DarkBlue,
            BasicColor::DarkMagenta => crossterm::style::Color::DarkMagenta,
            BasicColor::DarkCyan => crossterm::style::Color::DarkCyan,
            BasicColor::Grey => crossterm::style::Color::Grey,

            BasicColor::DarkGrey => crossterm::style::Color::DarkGrey,

            BasicColor::Red => crossterm::style::Color::Red,
            BasicColor::Green => crossterm::style::Color::Green,
            BasicColor::Yellow => crossterm::style::Color::Yellow,
            BasicColor::Blue => crossterm::style::Color::Blue,
            BasicColor::Magenta => crossterm::style::Color::Magenta,
            BasicColor::Cyan => crossterm::style::Color::Cyan,
            BasicColor::White => crossterm::style::Color::White,
        }
    }

    /// Create inquire compatible types
    ///
    /// Basically the same as `.to_crossterm()`, except that "light" colors are prefixed
    pub fn to_inquire(&self) -> inquire::ui::Color {
        match self {
            BasicColor::Black => inquire::ui::Color::Black,
            BasicColor::DarkRed => inquire::ui::Color::DarkRed,
            BasicColor::DarkGreen => inquire::ui::Color::DarkGreen,
            BasicColor::DarkYellow => inquire::ui::Color::DarkYellow,
            BasicColor::DarkBlue => inquire::ui::Color::DarkBlue,
            BasicColor::DarkMagenta => inquire::ui::Color::DarkMagenta,
            BasicColor::DarkCyan => inquire::ui::Color::DarkCyan,
            BasicColor::Grey => inquire::ui::Color::Grey,

            BasicColor::DarkGrey => inquire::ui::Color::DarkGrey,

            BasicColor::Red => inquire::ui::Color::LightRed,
            BasicColor::Green => inquire::ui::Color::LightGreen,
            BasicColor::Yellow => inquire::ui::Color::LightYellow,
            BasicColor::Blue => inquire::ui::Color::LightBlue,
            BasicColor::Magenta => inquire::ui::Color::LightMagenta,
            BasicColor::Cyan => inquire::ui::Color::LightCyan,
            BasicColor::White => inquire::ui::Color::White,
        }
    }
}

pub struct FloxColor {
    ansi256: u8,
    rgb: (u8, u8, u8),
    basic: BasicColor,
}

impl FloxColor {
    #[allow(dead_code)] // todo: discuss how/where to integrate colors
    pub fn to_crossterm(&self) -> Option<crossterm::style::Color> {
        match supports_color::on(supports_color::Stream::Stderr) {
            Some(supports_color::ColorLevel { has_16m: true, .. }) => {
                Some(crossterm::style::Color::Rgb {
                    r: self.rgb.0,
                    g: self.rgb.1,
                    b: self.rgb.2,
                })
            },
            Some(supports_color::ColorLevel { has_256: true, .. }) => {
                Some(crossterm::style::Color::AnsiValue(self.ansi256))
            },
            Some(supports_color::ColorLevel {
                has_basic: true, ..
            }) => Some(self.basic.to_crossterm()),
            _ => None,
        }
    }

    pub fn to_inquire(&self) -> Option<inquire::ui::Color> {
        match supports_color::on(supports_color::Stream::Stderr) {
            Some(supports_color::ColorLevel { has_16m: true, .. }) => {
                Some(inquire::ui::Color::Rgb {
                    r: self.rgb.0,
                    g: self.rgb.1,
                    b: self.rgb.2,
                })
            },
            Some(supports_color::ColorLevel { has_256: true, .. }) => {
                Some(inquire::ui::Color::AnsiValue(self.ansi256))
            },
            Some(supports_color::ColorLevel {
                has_basic: true, ..
            }) => Some(self.basic.to_inquire()),
            _ => None,
        }
    }
}

/// Should match the defaults in `activation-scripts`.
pub const INDIGO_300: FloxColor = FloxColor {
    ansi256: 141,
    rgb: (175, 135, 255),
    basic: BasicColor::DarkYellow,
};

/// Should match the defaults in `activation-scripts`.
pub const INDIGO_400: FloxColor = FloxColor {
    ansi256: 99,
    rgb: (135, 95, 255),
    basic: BasicColor::Blue,
};
