use core::fmt;

use uart_16550::{Config, Uart16550Tty, backend::PioBackend};
use spin::Mutex;
use lazy_static::lazy_static;

use crate::terminals::{PrintOptions, terminal_color::TerminalColor};

lazy_static! {
    pub static ref SERIAL1: Mutex<Uart16550Tty<PioBackend>> = Mutex::new(unsafe {
        Uart16550Tty::new_port(0x3F8, Config::default()).expect("failed to initialize UART")
    });
}

#[doc(hidden)]
pub fn write(options: PrintOptions, args: fmt::Arguments) {
    use core::fmt::Write;
    let mut writer = SERIAL1.lock();

    let styled = options.foreground.is_some() || options.background.is_some();


    if let Some(foreground_temp) = options.foreground {
        writer.write_str(foreground_temp.ansi_fg()).unwrap();
    }

    if let Some(background_temp) = options.background {
        writer.write_str(background_temp.ansi_bg()).unwrap();
    };

    writer.write_fmt(args).unwrap();

    if styled {
        writer.write_str("\x1b[0m").unwrap();
    }
}