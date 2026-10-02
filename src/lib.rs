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
pub mod devices;
pub mod memory;
pub mod old_memory;

#[cfg(test)]
use bootloader::{BootInfo, entry_point};
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



    unsafe { interrupts::PICS.lock().initialize() };
    x86_64::instructions::interrupts::enable();
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

#[cfg(test)]
entry_point!(test_kernel_main);

/// Entry point for `cargo test`
#[cfg(test)]
fn test_kernel_main(_boot_info: &'static BootInfo) -> ! {
    init();
    test_main();
    
    hlt_loop();
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    println!(TEST.fg(TerminalColor::Red); "[Failed]\n");
    println!(TEST.fg(TerminalColor::Red); "Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    
    hlt_loop();
}

pub fn panic_handler(info: &PanicInfo) -> ! {
    println!(EXCEPTION; "\n{}", info);
    
    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info);
}
