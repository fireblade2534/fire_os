#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(fire_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;

use bootloader::BootInfo;
use fire_os::{
    memory::{
        memory_manager::{memory_manager, ReMapError, TranslateError},
        page_table::PageTableFlags,
        physical_memory::MapError,
    },
    println,
};
use x86_64::VirtAddr;

const TEST_BASE: u64 = 0x0000_4000_0000_0000;
const PAGE_SIZE: u64 = 4096;

fn test_page(index: u64) -> VirtAddr {
    VirtAddr::new(TEST_BASE + index * PAGE_SIZE)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    fire_os::init(boot_info);

    test_main();

    fire_os::hlt_loop();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    fire_os::test_panic_handler(info)
}

#[test_case]
fn allocate_and_translate_page() {
    let virtual_address = test_page(0);

    let mut memory = memory_manager();

    memory
        .allocate_page(virtual_address, PageTableFlags::WRITABLE)
        .expect("failed to allocate page");

    let physical_frame = memory
        .translate_frame(virtual_address)
        .expect("newly allocated page did not translate");

    assert_eq!(
        physical_frame.as_u64() % PAGE_SIZE,
        0,
        "translated physical frame was not page aligned"
    );

    assert_eq!(
        memory.translate_address(virtual_address).unwrap(),
        physical_frame,
        "translation of a page-aligned virtual address did not equal its frame"
    );

    memory
        .deallocate_page(virtual_address)
        .expect("failed to clean up allocated page");
}

#[test_case]
fn translation_preserves_page_offset() {
    let virtual_page = test_page(1);
    let offset = 0x5a3u64;

    let mut memory = memory_manager();

    memory
        .allocate_page(virtual_page, PageTableFlags::WRITABLE)
        .expect("failed to allocate page");

    let physical_frame = memory
        .translate_frame(virtual_page)
        .expect("failed to translate allocated frame");

    let translated = memory
        .translate_address(virtual_page + offset)
        .expect("failed to translate address inside page");

    assert_eq!(
        translated.as_u64(),
        physical_frame.as_u64() + offset,
        "page offset was not preserved during translation"
    );

    memory
        .deallocate_page(virtual_page)
        .expect("failed to clean up allocated page");
}

#[test_case]
fn allocated_page_is_zeroed() {
    let virtual_address = test_page(2);

    {
        let mut memory = memory_manager();

        memory
            .allocate_page(virtual_address, PageTableFlags::WRITABLE)
            .expect("failed to allocate page");
    }

    let value = unsafe {
        virtual_address.as_ptr::<u64>().read_volatile()
    };

    assert_eq!(
        value,
        0,
        "newly allocated page was not zero initialized"
    );

    memory_manager()
        .deallocate_page(virtual_address)
        .expect("failed to clean up allocated page");
}

#[test_case]
fn mapped_page_can_be_written_and_read() {
    let virtual_address = test_page(3);
    const VALUE: u64 = 0xDEAD_BEEF_CAFE_BABE;

    {
        let mut memory = memory_manager();

        memory
            .allocate_page(virtual_address, PageTableFlags::WRITABLE)
            .expect("failed to allocate page");
    }

    unsafe {
        let ptr = virtual_address.as_mut_ptr::<u64>();

        ptr.write_volatile(VALUE);

        assert_eq!(
            ptr.read_volatile(),
            VALUE,
            "value read through mapping did not match value written"
        );
    }

    memory_manager()
        .deallocate_page(virtual_address)
        .expect("failed to clean up allocated page");
}

#[test_case]
fn allocating_same_page_twice_fails() {
    let virtual_address = test_page(4);

    let mut memory = memory_manager();

    memory
        .allocate_page(virtual_address, PageTableFlags::WRITABLE)
        .expect("failed initial page allocation");

    let result =
        memory.allocate_page(virtual_address, PageTableFlags::WRITABLE);

    assert_eq!(
        result,
        Err(MapError::AlreadyMapped),
        "allocating an already-present virtual page returned the wrong result"
    );

    memory
        .deallocate_page(virtual_address)
        .expect("failed to clean up allocated page");
}

#[test_case]
fn unmap_makes_translation_fail() {
    let virtual_address = test_page(5);

    let mut memory = memory_manager();

    memory
        .allocate_page(virtual_address, PageTableFlags::WRITABLE)
        .expect("failed to allocate page");

    let original_frame = memory
        .translate_frame(virtual_address)
        .expect("failed to translate allocated page");

    let old_entry = memory
        .unmap_page(virtual_address)
        .expect("failed to unmap page");

    assert_eq!(
        old_entry.physical_frame(),
        original_frame,
        "unmap returned the wrong original physical frame"
    );

    assert_eq!(
        memory.translate_frame(virtual_address),
        Err(TranslateError::NotMapped),
        "unmapped page still translated as present"
    );

    // Restore it so deallocation can also exercise the normal mapped path.
    memory
        .map_existing_frame(virtual_address)
        .expect("failed to restore unmapped page");

    memory
        .deallocate_page(virtual_address)
        .expect("failed to clean up restored page");
}

#[test_case]
fn remap_restores_same_frame_and_data() {
    let virtual_address = test_page(6);
    const VALUE: u64 = 0x1122_3344_5566_7788;

    {
        let mut memory = memory_manager();

        memory
            .allocate_page(virtual_address, PageTableFlags::WRITABLE)
            .expect("failed to allocate page");
    }

    unsafe {
        virtual_address
            .as_mut_ptr::<u64>()
            .write_volatile(VALUE);
    }

    let mut memory = memory_manager();

    let original_frame = memory
        .translate_frame(virtual_address)
        .expect("failed to translate allocated page");

    memory
        .unmap_page(virtual_address)
        .expect("failed to unmap page");

    assert_eq!(
        memory.translate_frame(virtual_address),
        Err(TranslateError::NotMapped),
        "page still translated after unmapping"
    );

    memory
        .map_existing_frame(virtual_address)
        .expect("failed to remap existing frame");

    let restored_frame = memory
        .translate_frame(virtual_address)
        .expect("remapped page did not translate");

    assert_eq!(
        restored_frame,
        original_frame,
        "remapping did not restore the original physical frame"
    );

    drop(memory);

    let restored_value = unsafe {
        virtual_address.as_ptr::<u64>().read_volatile()
    };

    assert_eq!(
        restored_value,
        VALUE,
        "physical frame contents were not preserved across unmap/remap"
    );

    memory_manager()
        .deallocate_page(virtual_address)
        .expect("failed to clean up remapped page");
}

#[test_case]
fn remapping_present_page_fails() {
    let virtual_address = test_page(7);

    let mut memory = memory_manager();

    memory
        .allocate_page(virtual_address, PageTableFlags::WRITABLE)
        .expect("failed to allocate page");

    assert_eq!(
        memory.map_existing_frame(virtual_address),
        Err(ReMapError::AlreadyMapped),
        "remapping an already-present page returned the wrong result"
    );

    memory
        .deallocate_page(virtual_address)
        .expect("failed to clean up allocated page");
}

#[test_case]
fn deallocate_after_unmap_frees_original_frame() {
    let virtual_address = test_page(8);
    let replacement_address = test_page(9);

    let mut memory = memory_manager();

    memory
        .allocate_page(virtual_address, PageTableFlags::WRITABLE)
        .expect("failed to allocate page");

    let original_frame = memory
        .translate_frame(virtual_address)
        .expect("failed to translate allocated page");

    memory
        .unmap_page(virtual_address)
        .expect("failed to unmap page");

    // This specifically tests that deallocation decodes the poisoned
    // non-present PTE and frees the REAL physical frame.
    memory
        .deallocate_page(virtual_address)
        .expect("failed to deallocate previously unmapped page");

    memory
        .allocate_page(replacement_address, PageTableFlags::WRITABLE)
        .expect("failed to allocate replacement page");

    let replacement_frame = memory
        .translate_frame(replacement_address)
        .expect("failed to translate replacement page");

    assert_eq!(
        replacement_frame,
        original_frame,
        "deallocating an unmapped page did not free its original physical frame"
    );

    memory
        .deallocate_page(replacement_address)
        .expect("failed to clean up replacement page");
}

#[test_case]
fn deallocated_frame_can_be_reused() {
    let first_virtual = test_page(10);
    let second_virtual = test_page(11);

    let mut memory = memory_manager();

    memory
        .allocate_page(first_virtual, PageTableFlags::WRITABLE)
        .expect("failed to allocate first page");

    let first_frame = memory
        .translate_frame(first_virtual)
        .expect("failed to translate first page");

    memory
        .deallocate_page(first_virtual)
        .expect("failed to deallocate first page");

    memory
        .allocate_page(second_virtual, PageTableFlags::WRITABLE)
        .expect("failed to allocate second page");

    let second_frame = memory
        .translate_frame(second_virtual)
        .expect("failed to translate second page");

    assert_eq!(
        second_frame,
        first_frame,
        "freed physical frame was not reused"
    );

    memory
        .deallocate_page(second_virtual)
        .expect("failed to clean up second page");
}

#[test_case]
fn multiple_pages_get_distinct_frames() {
    const COUNT: usize = 16;
    const START_INDEX: u64 = 32;

    let mut memory = memory_manager();

    let virtual_pages: [VirtAddr; COUNT] =
        core::array::from_fn(|i| test_page(START_INDEX + i as u64));

    for address in virtual_pages {
        memory
            .allocate_page(address, PageTableFlags::WRITABLE)
            .expect("failed to allocate page during multi-page test");
    }

    let physical_frames = virtual_pages.map(|address| {
        memory
            .translate_frame(address)
            .expect("failed to translate page during multi-page test")
    });

    for i in 0..COUNT {
        for j in (i + 1)..COUNT {
            assert_ne!(
                physical_frames[i],
                physical_frames[j],
                "two distinct virtual pages were backed by the same physical frame"
            );
        }
    }

    for address in virtual_pages {
        memory
            .deallocate_page(address)
            .expect("failed to clean up page during multi-page test");
    }
}

#[test_case]
fn allocation_across_level_1_table_boundary_works() {
    // A level-1 page table covers 512 * 4 KiB = 2 MiB.
    // These addresses sit on opposite sides of a 2 MiB boundary and therefore
    // require two different level-1 tables.
    let before_boundary =
        VirtAddr::new(TEST_BASE + 0x001f_f000);

    let after_boundary =
        VirtAddr::new(TEST_BASE + 0x0020_0000);

    let mut memory = memory_manager();

    memory
        .allocate_page(before_boundary, PageTableFlags::WRITABLE)
        .expect("failed to allocate page before level-1 boundary");

    memory
        .allocate_page(after_boundary, PageTableFlags::WRITABLE)
        .expect("failed to allocate page after level-1 boundary");

    let before_frame = memory
        .translate_frame(before_boundary)
        .expect("failed to translate page before boundary");

    let after_frame = memory
        .translate_frame(after_boundary)
        .expect("failed to translate page after boundary");

    assert_ne!(
        before_frame,
        after_frame,
        "pages across a level-1 table boundary shared a physical frame"
    );

    memory
        .deallocate_page(before_boundary)
        .expect("failed to clean up page before boundary");

    memory
        .deallocate_page(after_boundary)
        .expect("failed to clean up page after boundary");
}
