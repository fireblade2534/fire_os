#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(fire_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;

use bootloader::BootInfo;
use fire_os::{memory::physical_memory::PhysicalMemoryManager, println, terminals::VGA};

static PHYSICAL_MEMORY: Once<Mutex<PhysicalMemoryManager>> = Once::new();

fn physical_memory() -> MutexGuard<'static, PhysicalMemoryManager> {
    PHYSICAL_MEMORY
        .wait()
        .expect("physical memory manager not initialized")
        .lock()
}

#[unsafe(no_mangle)] // Don't mangle the name of this function
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    PHYSICAL_MEMORY.call_once(|| {
        Mutex::new(PhysicalMemoryManager::new(boot_info))
    });

    test_main();

    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    fire_os::test_panic_handler(info)
}

use spin::{Mutex, MutexGuard, Once};
use x86_64::PhysAddr;

#[test_case]
fn allocate_returns_frame() {
    let mut memory = physical_memory();

    let frame = memory
        .allocate_frame()
        .expect("failed to allocate physical frame");

    memory.free_frame(frame);
}

#[test_case]
fn allocated_frame_is_page_aligned() {
    let mut memory = physical_memory();

    let frame = memory
        .allocate_frame()
        .expect("failed to allocate physical frame");

    assert_eq!(
        frame.as_u64() % 4096,
        0,
        "allocated frame was not 4096-byte aligned"
    );

    memory.free_frame(frame);
}

#[test_case]
fn sequential_allocations_are_different() {
    let mut memory = physical_memory();

    let frame_a = memory.allocate_frame().unwrap();
    let frame_b = memory.allocate_frame().unwrap();

    assert_ne!(
        frame_a,
        frame_b,
        "allocator returned the same frame twice"
    );

    memory.free_frame(frame_a);
    memory.free_frame(frame_b);
}

#[test_case]
fn freed_frame_can_be_reallocated() {
    let mut memory = physical_memory();

    let frame = memory.allocate_frame().unwrap();

    memory.free_frame(frame);

    let frame_again = memory.allocate_frame().unwrap();

    assert_eq!(
        frame,
        frame_again,
        "freed frame was not reused"
    );

    memory.free_frame(frame_again);
}

#[test_case]
fn many_allocations_are_unique() {
    const COUNT: usize = 64;

    let mut memory = physical_memory();

    let frames: [PhysAddr; COUNT] = core::array::from_fn(|_| {
        memory
            .allocate_frame()
            .expect("ran out of frames during uniqueness test")
    });

    for i in 0..COUNT {
        for j in (i + 1)..COUNT {
            assert_ne!(
                frames[i],
                frames[j],
                "allocator returned duplicate frame"
            );
        }
    }

    for frame in frames {
        memory.free_frame(frame);
    }
}

#[test_case]
fn many_allocations_are_aligned() {
    const COUNT: usize = 64;

    let mut memory = physical_memory();

    let frames: [PhysAddr; COUNT] = core::array::from_fn(|_| {
        memory
            .allocate_frame()
            .expect("ran out of frames during alignment test")
    });

    for frame in frames {
        assert_eq!(
            frame.as_u64() % 4096,
            0,
            "frame {:#x} is not page aligned",
            frame.as_u64()
        );
    }

    for frame in frames {
        memory.free_frame(frame);
    }
}

#[test_case]
fn freeing_middle_frame_makes_it_available_again() {
    let mut memory = physical_memory();

    let frame_a = memory.allocate_frame().unwrap();
    let frame_b = memory.allocate_frame().unwrap();
    let frame_c = memory.allocate_frame().unwrap();

    memory.free_frame(frame_b);

    let replacement = memory.allocate_frame().unwrap();

    assert_eq!(
        replacement,
        frame_b,
        "allocator did not reuse the freed middle frame"
    );

    memory.free_frame(frame_a);
    memory.free_frame(replacement);
    memory.free_frame(frame_c);
}

#[test_case]
fn freed_frames_can_all_be_reused() {
    const COUNT: usize = 16;

    let mut memory = physical_memory();

    let first: [PhysAddr; COUNT] =
        core::array::from_fn(|_| memory.allocate_frame().unwrap());

    for frame in first {
        memory.free_frame(frame);
    }

    let second: [PhysAddr; COUNT] =
        core::array::from_fn(|_| memory.allocate_frame().unwrap());

    for i in 0..COUNT {
        assert_eq!(
            first[i],
            second[i],
            "allocation order changed after freeing all frames"
        );
    }

    for frame in second {
        memory.free_frame(frame);
    }
}