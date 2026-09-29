#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![feature(abi_x86_interrupt)]
#![reexport_test_harness_main = "test_main"]

pub mod gdt;
pub mod terminals;
pub mod qemu;
pub mod interrupts;

use terminals::terminal_color::TerminalColor;
use qemu::{QemuExitCode, exit_qemu};
use core::panic::PanicInfo;

use crate::terminals::{EXCEPTION, TEST};

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        print!(TEST; "{}...\t", core::any::type_name::<T>());
        self();
        println!(TEST.fg(TerminalColor::Green); "[Ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    println!(TEST; "Running {} tests", tests.len());
    for test in tests {
        test.run();
    }

    exit_qemu(QemuExitCode::Success);
}

pub fn init() {
    gdt::init_gdt();
    interrupts::init_idt();
}

/// Entry point for `cargo test`
#[cfg(test)]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    init();
    test_main();
    loop {}
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    println!(TEST.fg(TerminalColor::Red); "[Failed]\n");
    println!(TEST.fg(TerminalColor::Red); "Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    loop {}
}

pub fn panic_handler(info: &PanicInfo) -> ! {
    println!(EXCEPTION; "\n{}", info);
    loop {}
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info);
}
