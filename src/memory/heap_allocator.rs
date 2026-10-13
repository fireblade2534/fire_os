use core::{alloc::{GlobalAlloc, Layout}, ptr::null_mut};

use spin::Mutex;
use x86_64::VirtAddr;

use crate::memory::heap::Heap;

pub const HEAP_START: VirtAddr = VirtAddr::new(0x_4444_4444_0000);
pub const HEAP_SIZE: usize = 100 * 1024; 

#[global_allocator]
pub static KERNEL_ALLOCATOR: HeapAllocator = HeapAllocator::new(HEAP_START, HEAP_SIZE);

pub struct HeapAllocator {
    heap: Mutex<Heap>,
}

impl HeapAllocator {
    pub const fn new(heap_start: VirtAddr, heap_size: usize) -> Self {
        Self {
            heap: Mutex::new(Heap::new(heap_start, heap_size)),
        }
    }

    pub fn init_heap(&self) {
        self.heap.lock().init();
    }
}

unsafe impl GlobalAlloc for HeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.heap
            .lock()
            .allocate(layout)
            .unwrap_or(null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.heap.lock().deallocate(ptr, layout).unwrap_or_default();
    }
}