#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

use fire_os::terminals::EXCEPTION;
use fire_os::terminals::terminal_color::TerminalColor;
use fire_os::qemu::{QemuExitCode, exit_qemu};
use core::panic::PanicInfo;
use fire_os::{println};



#[unsafe(no_mangle)] // Don't mangle the name of this function
pub extern "C" fn _start() -> ! {
    fire_os::init();

    x86_64::instructions::interrupts::int3();

    println!("It did not crash!");
    
    loop {}
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    fire_os::panic_handler(info);
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    fire_os::test_panic_handler(info)
}