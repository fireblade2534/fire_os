
use lazy_static::lazy_static;
use spin::Mutex;
use volatile::Volatile;
use core::fmt;

use crate::{println, terminals::{PrintOptions, VGA, terminal_color::TerminalColor}};

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ColorCode(u8);

impl ColorCode {
    pub fn new(foreground: TerminalColor, background: TerminalColor) -> Self {
        Self {
            0: ((background as u8) << 4) | (foreground as u8)
        }
    }

    pub fn foreground(&self) -> TerminalColor {
        return TerminalColor::from_u8(self.0 & 0b1111);
    }

    pub fn background(&self) -> TerminalColor {
        return TerminalColor::from_u8((self.0 >> 4) & 0b1111)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct VGAScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

#[repr(transparent)]
struct VGABuffer {
    chars: [[Volatile<VGAScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

lazy_static! {
    pub static ref VGAWRITER: Mutex<VGAWriter> = Mutex::new(VGAWriter {
        row_position: 0,
        column_position: 0,
        color_code: ColorCode::new(TerminalColor::White, TerminalColor::Black),
        buffer: unsafe { &mut *(0xb8000 as *mut VGABuffer) },
    });
}

pub struct VGAWriter {
    row_position: usize,
    column_position: usize,
    color_code: ColorCode,
    buffer: &'static mut VGABuffer,
}

impl VGAWriter {
    pub fn set_color(&mut self, foreground: TerminalColor, background: TerminalColor) {
        self.color_code = ColorCode::new(foreground, background);
    }

    pub fn set_color_code(&mut self, color_code: ColorCode) {
        self.color_code = color_code;
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            match_byte=> {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = self.row_position;
                let col = self.column_position;
                
                self.buffer.chars[row][col].write(VGAScreenChar {
                    ascii_character: match_byte,
                    color_code: self.color_code,
                });

                self.column_position += 1;
            }
        }
    }

    pub fn write_string(&mut self, string: &str) {
        for byte in string.bytes() {
            match byte {
                // Printable ascii range + new line
                0x20..= 0x7e | b'\n' => self.write_byte(byte),

                // Not part of the printable ascii range
                _ => self.write_byte(0xfe),
            }
        }
    }

    pub fn clear_row(&mut self, row: usize) {
        let blank = VGAScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };

        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(blank);
        }
    }

    pub fn clear_screen(&mut self) {
        for row in 0..BUFFER_HEIGHT {
            self.clear_row(row);
        }

        self.row_position = 0;
        self.column_position = 0;
    }

    fn new_line(&mut self) {
        if self.row_position == BUFFER_HEIGHT - 1 {
            for row in 1..BUFFER_HEIGHT {
                for col in 0..BUFFER_WIDTH {
                    let character = self.buffer.chars[row][col].read();
                    self.buffer.chars[row - 1][col].write(character);
                }
            }

            self.clear_row(BUFFER_HEIGHT - 1);

            self.column_position = 0;
        } else {
            self.row_position += 1;
            self.column_position = 0;
        }
    }
}

impl fmt::Write for VGAWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);

        Ok(())
    }
}

#[doc(hidden)]
pub fn write(options: PrintOptions, args: fmt::Arguments) {
    use core::fmt::Write;
    let mut writer = VGAWRITER.lock();

    let old_color = writer.color_code;

    let foreground = if let Some(foreground_temp) = options.foreground {
        foreground_temp
    } else {
        old_color.foreground()
    };

    let background = if let Some(background_temp) = options.background {
        background_temp
    } else {
        old_color.background()
    };

    writer.set_color(foreground, background);
    writer.write_fmt(args).unwrap();

    writer.set_color_code(old_color);
}

pub fn set_color(foreground: TerminalColor, background: TerminalColor) {
    VGAWRITER.lock().set_color(foreground, background);
}

pub fn clear_screen() {
    VGAWRITER.lock().clear_screen();
}

#[test_case]
fn test_println_simple() {
    println!(VGA; "test_println_simple output");
}

#[test_case]
fn test_println_many() {
    for _ in 0..300 {
        println!(VGA; "test_println_many output");
    }
}

#[test_case]
fn test_println_output() {
    clear_screen();

    let s = "Some test string that fits on a single line";
    println!(VGA; "{}", s);
    for (i, c) in s.chars().enumerate() {
        let screen_char = VGAWRITER.lock().buffer.chars[0][i].read();
        assert_eq!(char::from(screen_char.ascii_character), c);
    }
}