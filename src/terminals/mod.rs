pub mod serial;
pub mod terminal_color;
pub mod vga_buffer;

use core::fmt;

use terminal_color::TerminalColor;

#[derive(Debug, Clone, Copy)]
pub enum Output {
    Auto,
    Vga,
    Serial,
    Both,
}

#[derive(Debug, Clone, Copy)]
pub enum PrintClass {
    Normal,
    Exception,
    Test,
}

#[derive(Debug, Clone, Copy)]
pub struct PrintOptions {
    pub output: Output,
    pub class: PrintClass,
    pub foreground: Option<TerminalColor>,
    pub background: Option<TerminalColor>,
}

impl PrintOptions {
    pub const fn new() -> Self {
        Self {
            output: Output::Auto,
            class: PrintClass::Normal,
            foreground: None,
            background: None,
        }
    }

    pub const fn output(mut self, output: Output) -> Self {
        self.output = output;
        self
    }

    pub const fn class(mut self, class: PrintClass) -> Self {
        self.class = class;
        self
    }

    pub const fn fg(mut self, color: TerminalColor) -> Self {
        self.foreground = Some(color);
        self
    }

    pub const fn bg(mut self, color: TerminalColor) -> Self {
        self.background = Some(color);
        self
    }
}

pub const DEFAULT: PrintOptions = PrintOptions::new();
pub const VGA: PrintOptions = PrintOptions::new().output(Output::Vga);
pub const SERIAL: PrintOptions = PrintOptions::new().output(Output::Serial);
pub const BOTH: PrintOptions = PrintOptions::new().output(Output::Both);
pub const EXCEPTION: PrintOptions = PrintOptions::new().class(PrintClass::Exception).fg(TerminalColor::Red);
pub const TEST: PrintOptions = PrintOptions::new().class(PrintClass::Test);

#[doc(hidden)]
pub fn default_output() -> Output {
    #[cfg(any(test, feature = "kernel-test"))]
    return Output::Vga;
    
    #[cfg(any(feature = "serial-output"))]
    return Output::Both;

    #[cfg(not(any(test, feature = "serial-output")))]
    return Output::Vga;
}

#[doc(hidden)]
pub fn resolve_output(options: PrintOptions) -> Output {
    match options.output {
        Output::Vga | Output::Serial | Output::Both => {
            return options.output;
        }
        Output::Auto => {}
    }

    match options.class {
        PrintClass::Test => Output::Serial,

        PrintClass::Exception => {
            #[cfg(any(feature = "kernel-test"))]
            return Output::Vga;

            #[cfg(not(any(feature = "kernel-test")))]
            return Output::Both;
        }

        PrintClass::Normal => default_output()
    }
}

#[doc(hidden)]
pub fn _print(options: PrintOptions, args: fmt::Arguments) {
    match resolve_output(options) {
        Output::Vga => {
            vga_buffer::write(options, args);
        }

        Output::Serial => {
            serial::write(options, args);
        }

        Output::Both => {
            vga_buffer::write(options, args);
            serial::write(options, args);
        }

        Output::Auto => unreachable!(),
    }
}

#[macro_export]
macro_rules! print {
    // Options
    ($options:expr; $($arg:tt)*) => {{
        $crate::terminals::_print(
            $options,
            format_args!($($arg)*),
        );
    }};

    // Normal
    ($($arg:tt)*) => {{
        $crate::terminals::_print(
            $crate::terminals::PrintOptions::new(),
            format_args!($($arg)*),
        );
    }};
}

#[macro_export]
macro_rules! println {
    () => {{
        $crate::print!("\n");
    }};

    ($options:expr; $($arg:tt)*) => {{
        $crate::print!(
            $options; "{}\n", format_args!($($arg)*)
        );
    }};

    ($options:expr;) => {{
        $crate::print!($options; "\n");
    }};

    ($($arg:tt)*) => {{
        $crate::print!(
            "{}\n",
            format_args!($($arg)*)
        );
    }};
}