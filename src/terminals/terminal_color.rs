#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TerminalColor {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}



impl TerminalColor {
    pub fn from_u8(value: u8) -> TerminalColor {
        if value > 15 {
            return TerminalColor::Black;
        }

        return unsafe {
            core::mem::transmute::<u8, TerminalColor>(value)
        }
    }

    pub const fn ansi_fg(self) -> &'static str {
        match self {
            TerminalColor::Black => "\x1b[30m",
            TerminalColor::Blue => "\x1b[34m",
            TerminalColor::Green => "\x1b[32m",
            TerminalColor::Cyan => "\x1b[36m",
            TerminalColor::Red => "\x1b[31m",
            TerminalColor::Magenta => "\x1b[35m",
            TerminalColor::Brown => "\x1b[33m",
            TerminalColor::LightGray => "\x1b[37m",
            TerminalColor::DarkGray => "\x1b[90m",
            TerminalColor::LightBlue => "\x1b[94m",
            TerminalColor::LightGreen => "\x1b[92m",
            TerminalColor::LightCyan => "\x1b[96m",
            TerminalColor::LightRed => "\x1b[91m",
            TerminalColor::Pink => "\x1b[95m",
            TerminalColor::Yellow => "\x1b[93m",
            TerminalColor::White => "\x1b[97m",
        }
    }

    pub const fn ansi_bg(self) -> &'static str {
        match self {
            TerminalColor::Black      => "\x1b[40m",
            TerminalColor::Blue       => "\x1b[44m",
            TerminalColor::Green      => "\x1b[42m",
            TerminalColor::Cyan       => "\x1b[46m",
            TerminalColor::Red        => "\x1b[41m",
            TerminalColor::Magenta    => "\x1b[45m",
            TerminalColor::Brown      => "\x1b[43m",
            TerminalColor::LightGray  => "\x1b[47m",
            TerminalColor::DarkGray   => "\x1b[100m",
            TerminalColor::LightBlue  => "\x1b[104m",
            TerminalColor::LightGreen => "\x1b[102m",
            TerminalColor::LightCyan  => "\x1b[106m",
            TerminalColor::LightRed   => "\x1b[101m",
            TerminalColor::Pink       => "\x1b[105m",
            TerminalColor::Yellow     => "\x1b[103m",
            TerminalColor::White      => "\x1b[107m",
        }
    }
}
