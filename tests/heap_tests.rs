#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(fire_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::{alloc::Layout, panic::PanicInfo, ptr};

use bootloader::BootInfo;
use fire_os::memory::{
    heap::Heap,
    memory_manager::{memory_manager, TranslateError},
};
use x86_64::VirtAddr;

// Keep these disjoint from the paging tests' TEST_BASE and from HEAP_START.
const TEST_BASE: u64 = 0x0000_5000_0000_0000;
const TEST_STRIDE: u64 = 2 * 1024 * 1024;
const PAGE_SIZE: usize = 4096;
const TEST_HEAP_SIZE: usize = 100 * 1024;

fn test_heap_address(index: u64) -> VirtAddr {
    VirtAddr::new(TEST_BASE + index * TEST_STRIDE)
}

fn layout(size: usize, align: usize) -> Layout {
    Layout::from_size_align(size, align).expect("invalid test allocation layout")
}

// Each test gets its own heap address range. The fixture returns mapped pages
// to the page manager, including the page immediately after the heap limit
// so a page-boundary regression doesn't leak a frame between tests.
struct HeapFixture {
    heap: Heap,
    start: VirtAddr,
}

impl HeapFixture {
    fn new(index: u64) -> Self {
        let start = test_heap_address(index);
        let mut heap = Heap::new(start, TEST_HEAP_SIZE);
        heap.init();
        Self { heap, start }
    }

    fn allocate(&mut self, size: usize, align: usize) -> *mut u8 {
        self.heap
            .allocate(layout(size, align))
            .expect("heap allocation unexpectedly failed")
    }

    fn deallocate(&mut self, ptr: *mut u8, size: usize, align: usize) {
        // The pointer/layout pairs used by the tests always come from a
        // successful allocation and are freed once.
        self.heap.deallocate(ptr, layout(size, align));
    }

    fn offset(&self, ptr: *mut u8) -> usize {
        (ptr as usize)
            .checked_sub(self.start.as_u64() as usize)
            .expect("allocation pointer is below heap start")
    }

    fn assert_fully_recovered(&mut self) {
        // After every live allocation has been freed, first-fit should again
        // be able to hand out the entire contiguous virtual heap range.
        let whole = self.allocate(TEST_HEAP_SIZE, 8);
        assert_eq!(
            whole as usize,
            self.start.as_u64() as usize,
            "free blocks did not coalesce back into the full heap"
        );
        self.deallocate(whole, TEST_HEAP_SIZE, 8);
    }
}

impl Drop for HeapFixture {
    fn drop(&mut self) {
        let mut memory = memory_manager();

        // The extra iteration cleans up an out-of-range page if the current
        // inclusive-end mapping implementation mapped one by mistake.
        for i in 0..=(TEST_HEAP_SIZE / PAGE_SIZE) {
            let address = self.start + (i * PAGE_SIZE) as u64;
            if memory.translate_frame(address).is_ok() {
                memory
                    .deallocate_page(address)
                    .expect("failed to clean up heap test page");
            }
        }
    }
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
fn heap_initialization_maps_first_page() {
    let fixture = HeapFixture::new(0);
    let memory = memory_manager();

    assert!(
        memory.translate_frame(fixture.start).is_ok(),
        "the initial heap page was not mapped"
    );
    assert_eq!(
        memory.translate_frame(fixture.start + PAGE_SIZE as u64),
        Err(TranslateError::NotMapped),
        "heap initialization unexpectedly mapped the next page"
    );
}

#[test_case]
fn first_allocation_starts_at_heap_start() {
    let mut fixture = HeapFixture::new(1);
    let first = fixture.allocate(64, 8);

    assert_eq!(
        first as usize,
        fixture.start.as_u64() as usize,
        "first allocation did not start at the beginning of the free heap"
    );

    fixture.deallocate(first, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn separate_allocations_have_nonoverlapping_addresses() {
    let mut fixture = HeapFixture::new(2);
    let a = fixture.allocate(64, 8);
    let b = fixture.allocate(128, 8);
    let c = fixture.allocate(256, 8);

    assert!(
        (b as usize) >= (a as usize) + 64,
        "allocations A and B overlap"
    );
    assert!(
        (c as usize) >= (b as usize) + 128,
        "allocations B and C overlap"
    );
    assert!(
        fixture.offset(c) + 256 <= TEST_HEAP_SIZE,
        "allocation extended beyond heap limit"
    );

    fixture.deallocate(b, 128, 8);
    fixture.deallocate(c, 256, 8);
    fixture.deallocate(a, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn allocations_respect_various_alignments() {
    let mut fixture = HeapFixture::new(3);
    let aligns = [1usize, 2, 8, 16, 32, 64, 256, 4096];
    let mut pointers = [ptr::null_mut::<u8>(); 8];

    for (i, align) in aligns.iter().copied().enumerate() {
        pointers[i] = fixture.allocate(33, align);
        assert_eq!(
            pointers[i] as usize % align,
            0,
            "heap returned a pointer with the wrong alignment"
        );
    }

    for i in (0..pointers.len()).rev() {
        fixture.deallocate(pointers[i], 33, aligns[i]);
    }
    fixture.assert_fully_recovered();
}

#[test_case]
fn large_alignment_preserves_free_prefix() {
    let mut fixture = HeapFixture::new(4);
    let first = fixture.allocate(24, 8);
    let page_aligned = fixture.allocate(32, 4096);

    assert_eq!(
        fixture.offset(page_aligned),
        PAGE_SIZE,
        "high-alignment allocation did not land at the next page boundary"
    );

    // First-fit should reuse the free prefix created before page_aligned.
    let from_prefix = fixture.allocate(64, 8);
    assert_eq!(
        fixture.offset(from_prefix),
        24,
        "allocator did not reuse the free region before the aligned block"
    );

    fixture.deallocate(page_aligned, 32, 4096);
    fixture.deallocate(first, 24, 8);
    fixture.deallocate(from_prefix, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn tiny_allocations_have_room_for_free_block_metadata() {
    let mut fixture = HeapFixture::new(5);
    let sizes = [1usize, 2, 3, 7, 9];
    let mut pointers = [ptr::null_mut::<u8>(); 5];

    for (i, size) in sizes.iter().copied().enumerate() {
        pointers[i] = fixture.allocate(size, 1);
        if i > 0 {
            assert!(
                (pointers[i] as usize)
                    >= (pointers[i - 1] as usize) + core::mem::size_of::<usize>() * 2,
                "tiny allocations did not reserve room for free-list metadata"
            );
        }
    }

    for i in (0..sizes.len()).rev() {
        fixture.deallocate(pointers[i], sizes[i], 1);
    }
    fixture.assert_fully_recovered();
}

#[test_case]
fn freeing_a_block_makes_it_reusable() {
    let mut fixture = HeapFixture::new(6);
    let a = fixture.allocate(64, 8);
    let b = fixture.allocate(128, 8);
    let c = fixture.allocate(64, 8);

    fixture.deallocate(b, 128, 8);
    let replacement = fixture.allocate(128, 8);
    assert_eq!(
        replacement, b,
        "same-sized allocation did not reuse the freed hole"
    );

    fixture.deallocate(c, 64, 8);
    fixture.deallocate(replacement, 128, 8);
    fixture.deallocate(a, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn first_fit_uses_earliest_suitable_free_block() {
    let mut fixture = HeapFixture::new(7);
    let a = fixture.allocate(64, 8);
    let b = fixture.allocate(128, 8);
    let c = fixture.allocate(64, 8);
    let d = fixture.allocate(256, 8);
    let e = fixture.allocate(64, 8);

    fixture.deallocate(d, 256, 8);
    fixture.deallocate(b, 128, 8);
    let replacement = fixture.allocate(96, 8);
    assert_eq!(
        replacement, b,
        "first-fit allocation skipped the earliest suitable free block"
    );

    fixture.deallocate(replacement, 96, 8);
    fixture.deallocate(c, 64, 8);
    fixture.deallocate(e, 64, 8);
    fixture.deallocate(a, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn freeing_first_block_updates_free_list_head() {
    let mut fixture = HeapFixture::new(8);
    let a = fixture.allocate(64, 8);
    let b = fixture.allocate(64, 8);

    fixture.deallocate(a, 64, 8);
    let reused = fixture.allocate(32, 8);
    assert_eq!(
        reused, a,
        "a freed block at heap start was not reinserted into the free list"
    );

    fixture.deallocate(reused, 32, 8);
    fixture.deallocate(b, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn freeing_middle_block_preserves_live_neighbors() {
    let mut fixture = HeapFixture::new(9);
    let a = fixture.allocate(64, 8);
    let b = fixture.allocate(128, 8);
    let c = fixture.allocate(64, 8);

    unsafe {
        a.write_volatile(0xA5);
        c.write_volatile(0x5A);
    }

    fixture.deallocate(b, 128, 8);
    let reused = fixture.allocate(64, 8);
    assert_eq!(reused, b, "allocator did not reuse a freed middle block");

    unsafe {
        assert_eq!(a.read_volatile(), 0xA5, "left neighbor was corrupted");
        assert_eq!(c.read_volatile(), 0x5A, "right neighbor was corrupted");
    }

    fixture.deallocate(a, 64, 8);
    fixture.deallocate(c, 64, 8);
    fixture.deallocate(reused, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn freeing_last_allocation_merges_with_trailing_space() {
    let mut fixture = HeapFixture::new(10);
    let a = fixture.allocate(64, 8);
    let b = fixture.allocate(64, 8);
    let c = fixture.allocate(64, 8);

    fixture.deallocate(c, 64, 8);
    let larger = fixture.allocate(4096, 8);
    assert_eq!(
        larger, c,
        "last allocation was not merged with the free region after it"
    );

    fixture.deallocate(larger, 4096, 8);
    fixture.deallocate(b, 64, 8);
    fixture.deallocate(a, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn freeing_in_reverse_order_coalesces_everything() {
    let mut fixture = HeapFixture::new(11);
    const COUNT: usize = 8;
    let mut pointers = [ptr::null_mut::<u8>(); COUNT];
    let sizes: [usize; COUNT] = core::array::from_fn(|i| 32 + i * 24);

    for i in 0..COUNT {
        pointers[i] = fixture.allocate(sizes[i], 8);
    }
    for i in (0..COUNT).rev() {
        fixture.deallocate(pointers[i], sizes[i], 8);
    }

    fixture.assert_fully_recovered();
}

#[test_case]
fn freeing_in_shuffled_order_coalesces_both_neighbors() {
    let mut fixture = HeapFixture::new(12);
    const COUNT: usize = 6;
    let mut pointers = [ptr::null_mut::<u8>(); COUNT];

    for entry in &mut pointers {
        *entry = fixture.allocate(96, 8);
    }

    // Ensures insertion at both list head and middle, and coalescing on both sides.
    for i in [1usize, 4, 0, 3, 5, 2] {
        fixture.deallocate(pointers[i], 96, 8);
    }

    fixture.assert_fully_recovered();
}

#[test_case]
fn large_allocation_demand_maps_multiple_pages() {
    let mut fixture = HeapFixture::new(13);
    let size = PAGE_SIZE * 4 + 257;
    let buffer = fixture.allocate(size, 8);

    unsafe {
        for offset in [0, PAGE_SIZE - 1, PAGE_SIZE, PAGE_SIZE * 2, size - 1] {
            buffer.add(offset).write_volatile((offset % 251) as u8);
        }
        for offset in [0, PAGE_SIZE - 1, PAGE_SIZE, PAGE_SIZE * 2, size - 1] {
            assert_eq!(
                buffer.add(offset).read_volatile(),
                (offset % 251) as u8,
                "allocation was not readable/writable across a page boundary"
            );
        }
    }

    let memory = memory_manager();
    for i in 0..=4usize {
        assert!(
            memory
                .translate_frame(fixture.start + (i * PAGE_SIZE) as u64)
                .is_ok(),
            "a physical page required by a large allocation was not mapped"
        );
    }
    drop(memory);

    fixture.deallocate(buffer, size, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn heap_virtual_pages_use_distinct_physical_frames() {
    let mut fixture = HeapFixture::new(14);
    let size = PAGE_SIZE * 3;
    let buffer = fixture.allocate(size, 8);

    let memory = memory_manager();
    let frames = core::array::from_fn::<_, 3, _>(|i| {
        memory
            .translate_frame(fixture.start + (i * PAGE_SIZE) as u64)
            .expect("large allocation did not map a required page")
    });

    for i in 0..frames.len() {
        for j in (i + 1)..frames.len() {
            assert_ne!(
                frames[i], frames[j],
                "two heap virtual pages were backed by one physical frame"
            );
        }
    }
    drop(memory);

    fixture.deallocate(buffer, size, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn allocating_more_blocks_does_not_corrupt_existing_data() {
    let mut fixture = HeapFixture::new(15);
    let a = fixture.allocate(64, 8);
    let b = fixture.allocate(PAGE_SIZE * 2 + 99, 16);
    let c = fixture.allocate(128, 64);

    unsafe {
        a.write_volatile(0xA1);
        b.write_volatile(0xB2);
        b.add(PAGE_SIZE * 2 + 98).write_volatile(0xC3);
        c.write_volatile(0xD4);
    }

    let d = fixture.allocate(PAGE_SIZE + 33, 32);
    unsafe {
        assert_eq!(a.read_volatile(), 0xA1, "allocation A was corrupted");
        assert_eq!(b.read_volatile(), 0xB2, "allocation B was corrupted");
        assert_eq!(
            b.add(PAGE_SIZE * 2 + 98).read_volatile(),
            0xC3,
            "end of allocation B was corrupted"
        );
        assert_eq!(c.read_volatile(), 0xD4, "allocation C was corrupted");
    }

    fixture.deallocate(d, PAGE_SIZE + 33, 32);
    fixture.deallocate(b, PAGE_SIZE * 2 + 99, 16);
    fixture.deallocate(a, 64, 8);
    fixture.deallocate(c, 128, 64);
    fixture.assert_fully_recovered();
}

#[test_case]
fn request_larger_than_heap_fails_without_corruption() {
    let mut fixture = HeapFixture::new(16);
    let a = fixture.allocate(64, 8);
    unsafe { a.write_volatile(0x7C) };

    assert!(
        fixture.heap.allocate(layout(TEST_HEAP_SIZE + 1, 8)).is_err(),
        "heap accepted a request larger than its reserved address range"
    );

    unsafe {
        assert_eq!(
            a.read_volatile(),
            0x7C,
            "failed allocation corrupted an existing allocation"
        );
    }
    fixture.deallocate(a, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn full_heap_allocation_exhausts_free_list() {
    let mut fixture = HeapFixture::new(17);
    let whole = fixture.allocate(TEST_HEAP_SIZE, 8);

    assert_eq!(fixture.offset(whole), 0, "full allocation did not use the heap start");
    assert!(
        fixture.heap.allocate(layout(16, 8)).is_err(),
        "allocator handed out memory after all heap bytes were allocated"
    );

    fixture.deallocate(whole, TEST_HEAP_SIZE, 8);
    let again = fixture.allocate(64, 8);
    assert_eq!(again, whole, "freeing the full heap did not make it reusable");
    fixture.deallocate(again, 64, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn fragmented_heap_rejects_request_larger_than_any_hole() {
    let mut fixture = HeapFixture::new(18);
    let part = 32 * 1024;
    let a = fixture.allocate(part, 8);
    let b = fixture.allocate(part, 8);
    let c = fixture.allocate(part, 8);

    fixture.deallocate(b, part, 8);

    // 32 KiB hole + 4 KiB free tail, separated by the live C allocation.
    assert!(
        fixture.heap.allocate(layout(40 * 1024, 8)).is_err(),
        "allocation incorrectly combined non-adjacent free regions"
    );

    fixture.deallocate(c, part, 8);
    fixture.deallocate(a, part, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn allocation_ending_at_page_boundary_is_writable() {
    let mut fixture = HeapFixture::new(19);
    let buffer = fixture.allocate(PAGE_SIZE, 8);

    unsafe {
        buffer.write_volatile(0x12);
        buffer.add(PAGE_SIZE - 1).write_volatile(0x34);
        assert_eq!(buffer.read_volatile(), 0x12);
        assert_eq!(buffer.add(PAGE_SIZE - 1).read_volatile(), 0x34);
    }

    fixture.deallocate(buffer, PAGE_SIZE, 8);
    fixture.assert_fully_recovered();
}

#[test_case]
fn allocation_does_not_map_outside_heap_limit() {
    let mut fixture = HeapFixture::new(20);
    let whole = fixture.allocate(TEST_HEAP_SIZE, 8);
    let first_address_after_heap = fixture.start + TEST_HEAP_SIZE as u64;

    assert_eq!(
        memory_manager().translate_frame(first_address_after_heap),
        Err(TranslateError::NotMapped),
        "allocating through the heap limit mapped a page outside the heap"
    );

    fixture.deallocate(whole, TEST_HEAP_SIZE, 8);
}
