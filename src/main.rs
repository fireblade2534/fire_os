#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

use bootloader::{BootInfo, entry_point};
use fire_os::terminals::EXCEPTION;
use fire_os::terminals::terminal_color::TerminalColor;
use fire_os::qemu::{QemuExitCode, exit_qemu};
use x86_64::VirtAddr;
use x86_64::structures::paging::Translate;
use core::panic::PanicInfo;
use fire_os::{memory, println};



entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    fire_os::init();
    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mapper = unsafe { memory::init(phys_mem_offset) };

    let addresses = [
        // the identity-mapped vga buffer page
        0xb8000,
        // some code page
        0x201008,
        // some stack page
        0x0100_0020_1a10,
        // virtual address mapped to physical address 0
        boot_info.physical_memory_offset,
    ];

    for &address in &addresses {
        let virt = VirtAddr::new(address);
        let phys = mapper.translate_addr(virt);
        println!("{:?} -> {:?}", virt, phys);
    }

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