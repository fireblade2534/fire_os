#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

use bootloader::{BootInfo, entry_point};
use fire_os::memory::memory_manager::{memory_manager};
use fire_os::terminals::EXCEPTION;
use fire_os::terminals::terminal_color::TerminalColor;
use fire_os::qemu::{QemuExitCode, exit_qemu};
use x86_64::{PhysAddr, VirtAddr};
use x86_64::structures::paging::Translate;
use core::panic::PanicInfo;
use fire_os::{memory, println};



entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    fire_os::init(boot_info);

    println!("KERNEL :D!");

    fire_os::hlt_loop();
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