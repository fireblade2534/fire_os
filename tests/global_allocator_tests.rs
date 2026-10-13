#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(fire_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::{
    alloc::{alloc, alloc_zeroed, dealloc, realloc},
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    string::String,
    sync::Arc,
    vec,
    vec::Vec,
};
use bootloader::BootInfo;
use core::{alloc::Layout, cell::Cell, panic::PanicInfo, ptr};
use fire_os::memory::heap_allocator::{HEAP_SIZE, HEAP_START};

const PAGE_SIZE: usize = 4096;

fn layout(size: usize, align: usize) -> Layout {
    Layout::from_size_align(size, align).expect("invalid allocation layout in test")
}

fn assert_inside_kernel_heap(ptr: *mut u8, size: usize) {
    let start = HEAP_START.as_u64() as usize;
    let end = start + HEAP_SIZE;
    let address = ptr as usize;
    assert!(address >= start, "allocation starts before the kernel heap");
    assert!(
        address.checked_add(size).is_some_and(|allocation_end| allocation_end <= end),
        "allocation extends past the kernel heap limit"
    );
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
fn box_allocates_and_reads_value() {
    let mut value = Box::new(0xDEAD_BEEF_CAFE_BABEu64);
    assert_eq!(*value, 0xDEAD_BEEF_CAFE_BABE);

    *value = 0x1122_3344_5566_7788;
    assert_eq!(*value, 0x1122_3344_5566_7788);
    assert_inside_kernel_heap((&mut *value as *mut u64).cast(), size_of::<u64>());
}

struct DropFlag<'a>(&'a Cell<bool>);

impl Drop for DropFlag<'_> {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test_case]
fn box_drop_runs_destructor() {
    let dropped = Cell::new(false);
    {
        let value = Box::new(DropFlag(&dropped));
        assert!(!dropped.get(), "destructor ran before Box was dropped");
        drop(value);
    }
    assert!(dropped.get(), "Box destructor did not run");
}

#[repr(align(256))]
struct AlignedObject {
    bytes: [u8; 128],
}

#[test_case]
fn box_respects_type_alignment() {
    let mut value = Box::new(AlignedObject { bytes: [0; 128] });
    let address = (&*value as *const AlignedObject) as usize;
    assert_eq!(address % 256, 0, "boxed object was not 256-byte aligned");
    value.bytes[0] = 0x32;
    value.bytes[127] = 0xAB;
    assert_eq!(value.bytes[0], 0x32);
    assert_eq!(value.bytes[127], 0xAB);
}

#[test_case]
fn vec_push_grows_and_preserves_values() {
    let mut numbers = Vec::new();
    for i in 0..2048u32 {
        numbers.push(i ^ 0x5A5A);
    }
    assert_eq!(numbers.len(), 2048);
    for i in 0..2048 {
        assert_eq!(numbers[i], (i as u32) ^ 0x5A5A, "Vec data corrupted during growth");
    }
}

#[test_case]
fn vec_resize_initializes_new_elements() {
    let mut data = vec![0x12u8; 128];
    data.resize(4096, 0xC3);
    assert_eq!(data.len(), 4096);
    assert!(data[..128].iter().all(|&x| x == 0x12));
    assert!(data[128..].iter().all(|&x| x == 0xC3));
}

#[test_case]
fn vec_spanning_pages_is_writable() {
    const COUNT: usize = PAGE_SIZE * 4 + 17;
    let mut data = vec![0u8; COUNT];
    let offsets = [0, PAGE_SIZE - 1, PAGE_SIZE, PAGE_SIZE + 1,
        PAGE_SIZE * 2, PAGE_SIZE * 3, COUNT - 1];

    for (index, offset) in offsets.iter().copied().enumerate() {
        data[offset] = (index as u8) + 1;
    }
    for (index, offset) in offsets.iter().copied().enumerate() {
        assert_eq!(data[offset], (index as u8) + 1,
            "write or read failed in a multi-page Vec");
    }
}

#[test_case]
fn vec_truncate_and_reuse() {
    let mut values: Vec<u64> = (0..512u64).collect();
    values.truncate(64);
    assert_eq!(values.len(), 64);
    for i in 0..64 {
        assert_eq!(values[i], i as u64);
    }
    values.extend(0..256u64);
    assert_eq!(values.len(), 320);
    assert_eq!(values[64], 0);
    assert_eq!(values[319], 255);
}

#[test_case]
fn string_grows_and_preserves_utf8() {
    let mut text = String::from("kernel");
    text.push_str(" heap allocation");
    text.push(' ');
    text.push('🦀');
    assert_eq!(text.as_str(), "kernel heap allocation 🦀");
    assert_eq!(text.chars().last(), Some('🦀'));
}

#[test_case]
fn multiple_heap_strings_keep_their_contents() {
    let mut strings = Vec::new();
    for i in 0..48 {
        let mut s = String::from("allocation:");
        for _ in 0..i {
            s.push('x');
        }
        strings.push(s);
    }
    for (i, text) in strings.iter().enumerate() {
        assert_eq!(text.len(), "allocation:".len() + i);
        assert!(text.starts_with("allocation:"));
        assert!(text["allocation:".len()..].bytes().all(|b| b == b'x'));
    }
}

#[test_case]
fn arc_clone_preserves_shared_allocation() {
    let first = Arc::new([11u64, 22, 33, 44]);
    let second = Arc::clone(&first);
    let third = Arc::clone(&first);
    assert_eq!(Arc::strong_count(&first), 3);
    assert_eq!(second[2], 33);
    assert_eq!(third[3], 44);
    drop(second);
    drop(third);
    assert_eq!(Arc::strong_count(&first), 1);
}

#[test_case]
fn btree_map_allocates_and_frees_nodes() {
    let mut values = BTreeMap::new();
    for i in (0..128u32).rev() {
        assert_eq!(values.insert(i, i * 7), None);
    }
    assert_eq!(values.len(), 128);
    for i in 0..128u32 {
        assert_eq!(values.get(&i), Some(&(i * 7)));
    }
    for i in (0..128u32).step_by(2) {
        assert_eq!(values.remove(&i), Some(i * 7));
    }
    assert_eq!(values.len(), 64);
    for i in (1..128u32).step_by(2) {
        assert_eq!(values.get(&i), Some(&(i * 7)));
    }
}

#[test_case]
fn btree_set_handles_repeated_insertions() {
    let mut values = BTreeSet::new();
    for i in 0..96u32 {
        assert!(values.insert(i));
        assert!(!values.insert(i));
    }
    assert_eq!(values.len(), 96);
    for i in 0..96u32 {
        assert!(values.contains(&i));
    }
}

#[test_case]
fn raw_alloc_returns_correct_alignment() {
    for align in [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 4096] {
        let request = layout(73, align);
        let address = unsafe { alloc(request) };
        assert!(!address.is_null(), "raw allocation failed");
        assert_eq!((address as usize) % align, 0, "raw allocation violated alignment");
        assert_inside_kernel_heap(address, request.size());
        unsafe { dealloc(address, request) };
    }
}

#[test_case]
fn raw_allocation_memory_can_be_written_and_read() {
    let request = layout(3 * PAGE_SIZE + 19, 16);
    let address = unsafe { alloc(request) };
    assert!(!address.is_null(), "multi-page raw allocation failed");
    assert_inside_kernel_heap(address, request.size());

    unsafe {
        ptr::write_bytes(address, 0xA5, request.size());
        for offset in [0, PAGE_SIZE - 1, PAGE_SIZE, 2 * PAGE_SIZE,
            3 * PAGE_SIZE - 1, request.size() - 1] {
            assert_eq!(address.add(offset).read_volatile(), 0xA5,
                "raw allocation lost data across a page boundary");
        }
        dealloc(address, request);
    }
}

#[test_case]
fn raw_alloc_zeroed_clears_reused_memory() {
    let request = layout(128, 8);
    let dirty = unsafe { alloc(request) };
    assert!(!dirty.is_null(), "initial allocation failed");
    unsafe {
        ptr::write_bytes(dirty, 0xA5, request.size());
        dealloc(dirty, request);
    }

    let zeroed = unsafe { alloc_zeroed(request) };
    assert!(!zeroed.is_null(), "zeroed allocation failed");
    for i in 0..request.size() {
        assert_eq!(unsafe { zeroed.add(i).read() }, 0,
            "alloc_zeroed returned nonzero memory");
    }
    unsafe { dealloc(zeroed, request) };
}

#[test_case]
fn raw_realloc_grow_preserves_old_contents() {
    let old_layout = layout(128, 16);
    let first = unsafe { alloc(old_layout) };
    assert!(!first.is_null(), "initial realloc buffer allocation failed");
    unsafe {
        for i in 0..128 {
            first.add(i).write(i as u8);
        }
        let grown = realloc(first, old_layout, 6000);
        assert!(!grown.is_null(), "realloc growth failed");
        assert_eq!((grown as usize) % 16, 0, "realloc lost the original alignment");
        for i in 0..128 {
            assert_eq!(grown.add(i).read(), i as u8,
                "realloc growth did not preserve existing data");
        }
        grown.add(5999).write(0x7E);
        assert_eq!(grown.add(5999).read(), 0x7E);
        dealloc(grown, layout(6000, 16));
    }
}

#[test_case]
fn raw_realloc_shrink_preserves_prefix() {
    let old_layout = layout(2048, 8);
    let first = unsafe { alloc(old_layout) };
    assert!(!first.is_null(), "initial shrink buffer allocation failed");
    unsafe {
        for i in 0..64 {
            first.add(i).write((i as u8) ^ 0xE3);
        }
        let shrunk = realloc(first, old_layout, 64);
        assert!(!shrunk.is_null(), "realloc shrink failed");
        for i in 0..64 {
            assert_eq!(shrunk.add(i).read(), (i as u8) ^ 0xE3,
                "realloc shrink did not preserve the prefix");
        }
        dealloc(shrunk, layout(64, 8));
    }
}

#[test_case]
fn simultaneous_raw_allocations_do_not_overlap() {
    const COUNT: usize = 64;
    let request = layout(48, 16);
    let mut addresses = [ptr::null_mut::<u8>(); COUNT];

    for (i, slot) in addresses.iter_mut().enumerate() {
        let allocated = unsafe { alloc(request) };
        assert!(!allocated.is_null(), "one of the simultaneous allocations failed");
        unsafe { ptr::write_bytes(allocated, i as u8, request.size()) };
        *slot = allocated;
    }

    for i in 0..COUNT {
        let start_i = addresses[i] as usize;
        let end_i = start_i + request.size();
        for j in (i + 1)..COUNT {
            let start_j = addresses[j] as usize;
            let end_j = start_j + request.size();
            assert!(end_i <= start_j || end_j <= start_i,
                "two live allocations overlap");
        }
        for byte in 0..request.size() {
            assert_eq!(unsafe { addresses[i].add(byte).read() }, i as u8,
                "one allocation overwrote another");
        }
    }

    for address in addresses.into_iter().rev() {
        unsafe { dealloc(address, request) };
    }
}

#[test_case]
fn freed_raw_block_can_be_reused() {
    let request = layout(96, 16);
    let first = unsafe { alloc(request) };
    let second = unsafe { alloc(request) };
    assert!(!first.is_null() && !second.is_null(), "initial allocations failed");
    unsafe { dealloc(first, request) };

    // No intervening allocations: your first-fit free list should use this hole.
    let replacement = unsafe { alloc(request) };
    assert_eq!(replacement, first, "freed first-fit block was not reused");

    unsafe {
        dealloc(replacement, request);
        dealloc(second, request);
    }
}

#[test_case]
fn freeing_middle_allocation_preserves_neighbors() {
    let request = layout(256, 16);
    let a = unsafe { alloc(request) };
    let b = unsafe { alloc(request) };
    let c = unsafe { alloc(request) };
    assert!(!a.is_null() && !b.is_null() && !c.is_null());

    unsafe {
        ptr::write_bytes(a, 0xAA, request.size());
        ptr::write_bytes(c, 0xCC, request.size());
        dealloc(b, request);
        let d = alloc(request);
        assert!(!d.is_null(), "reallocation into middle hole failed");
        ptr::write_bytes(d, 0xDD, request.size());
        for i in 0..request.size() {
            assert_eq!(a.add(i).read(), 0xAA, "first neighbor was overwritten");
            assert_eq!(c.add(i).read(), 0xCC, "last neighbor was overwritten");
        }
        dealloc(d, request);
        dealloc(a, request);
        dealloc(c, request);
    }
}

#[test_case]
fn repeated_raw_alloc_free_cycles_do_not_exhaust_heap() {
    let request = layout(512, 32);
    for i in 0..1000 {
        let allocated = unsafe { alloc(request) };
        assert!(!allocated.is_null(), "repeated alloc/dealloc exhausted the heap");
        unsafe {
            ptr::write_bytes(allocated, (i & 0xFF) as u8, request.size());
            assert_eq!(allocated.read(), (i & 0xFF) as u8);
            dealloc(allocated, request);
        }
    }
}

#[test_case]
fn failed_vec_reservation_preserves_existing_data() {
    let mut values = vec![0x7Bu8; 64];

    // This request is larger than the entire configured kernel heap.
    // try_reserve_exact returns an error instead of invoking OOM handling.
    assert!(
        values.try_reserve_exact(HEAP_SIZE * 4).is_err(),
        "oversized Vec reservation unexpectedly succeeded"
    );
    assert_eq!(values.len(), 64, "failed reservation changed Vec length");
    assert!(values.iter().all(|&x| x == 0x7B),
        "failed reservation corrupted the existing Vec");
    values.push(0x42);
    assert_eq!(values[64], 0x42,
        "allocator was not usable after a failed reservation");
}

#[test_case]
fn mixed_allocating_types_can_coexist() {
    let number = Box::new(12345u64);
    let mut vector = Vec::new();
    for i in 0..256 {
        vector.push(i * 3);
    }
    let message = String::from("simultaneous kernel allocations");
    let shared = Arc::new(0xCAFEu32);

    assert_eq!(*number, 12345);
    assert_eq!(vector[255], 765);
    assert_eq!(message, "simultaneous kernel allocations");
    assert_eq!(*shared, 0xCAFE);

    drop(vector);
    drop(message);
    drop(number);
    drop(shared);

    let after = Box::new(99u64);
    assert_eq!(*after, 99, "allocator failed after mixed allocations were dropped");
}
