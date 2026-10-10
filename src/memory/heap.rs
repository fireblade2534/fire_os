use core::{alloc::Layout, ptr::{NonNull, slice_from_raw_parts_mut}};

use pc_keyboard::KeyCode::M;
use x86_64::VirtAddr;

use crate::memory::{memory_manager::{PAGE_SIZE, memory_manager}, page_table::PageTableFlags};

pub const HEAP_START: VirtAddr = VirtAddr::new(0x_4444_4444_0000);
pub const HEAP_SIZE: usize = 100 * 1024; 

fn align_up(value: usize, align: usize) -> usize {
    assert_eq!(align & (align - 1), 0);

    return (value + (align - 1)) & !(align - 1)
}

pub struct Heap {
    start_address: VirtAddr,
    start_block: Option<NonNull<FreeBlock>>,

    mapped_page_end: VirtAddr,

    heap_size: usize
}

#[derive(Clone, Copy)]
pub struct FreeBlock {
    size: usize,
    next: Option<NonNull<FreeBlock>>,
}

impl Heap {
    pub fn new(heap_start: VirtAddr, heap_size: usize) -> Self {
        Self {
            start_address: heap_start,
            start_block: None,
            mapped_page_end: heap_start,
            heap_size: heap_size,
        }
    }

    fn write_free_block(&mut self, address: VirtAddr, size: usize, next: Option<NonNull<FreeBlock>>) -> NonNull<FreeBlock> {
        assert!(size >= size_of::<FreeBlock>());
        assert_eq!(address.as_u64() as usize % align_of::<FreeBlock>(), 0);

        let block_ptr = address.as_mut_ptr::<FreeBlock>();

        unsafe {
            block_ptr.write(FreeBlock {
                size: size,
                next: next,
            });

            return NonNull::new_unchecked(block_ptr);
        }
    }

    pub fn init(&mut self) {
        let mut memory = memory_manager();

        memory.allocate_page(self.start_address, PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE).expect("Failed to allocate inital page for the heap");
        self.mapped_page_end += PAGE_SIZE;

        self.start_block = Some(self.write_free_block(self.start_address, self.heap_size, None));
    }

    fn check_layout_fit(&self, frame_address: usize, frame: &FreeBlock, layout: Layout) -> Option<(Option<FreeBlock>, usize, Option<FreeBlock>)> {

        let frame_object_size = size_of::<FreeBlock>();
        let frame_object_align = align_of::<FreeBlock>();

        let layout_align = layout.align().max(frame_object_align);

        if frame.size >= layout.size() {

            let mut start_block: Option<FreeBlock> = None;
            let mut end_block: Option<FreeBlock> = None;

            let allocation_size = layout.size().max(frame_object_size);

            let mut start_offset = align_up(frame_address, layout_align) - frame_address;

            if start_offset != 0 {
                let remaining_prefix = frame_object_size.saturating_sub(start_offset);

                if remaining_prefix != 0 {
                    start_offset += remaining_prefix.div_ceil(layout_align) * layout_align;
                }

                start_block = Some(FreeBlock { size: start_offset, next: None })
            }

            let layout_end_address = align_up(frame_address + allocation_size + start_offset, frame_object_align);
            let layout_size = layout_end_address - frame_address;

            if frame.size < layout_size {
                return None;
            }

            let remaining_suffix = frame.size - layout_size;
            
            if remaining_suffix != 0 {
                if remaining_suffix < frame_object_size {
                    return None;
                }

                end_block = Some(FreeBlock { size: remaining_suffix, next: None })
            }

            return Some((start_block, layout_end_address, end_block));
        }

        return None;
    }

    fn clear_range(&mut self, start_address: usize, size: usize) {
        unsafe {
            core::intrinsics::volatile_set_memory(
                start_address as *mut u8,
                0,
                size,
            );
        }
    }

    pub fn allocate(&mut self, layout: Layout) -> Result<*mut u8, ()> {

        let mut link: *mut Option<NonNull<FreeBlock>> = &raw mut self.start_block;
        loop {
            let Some(current) = (unsafe { *link }) else {
                return Err(());
            };

            let frame = unsafe { *current.as_ptr() };
            let frame_start = current.as_ptr() as usize;

            let blocks = self.check_layout_fit(frame_start, &frame, layout);

            if let Some(blocks) = blocks {
                let alloc_start = frame_start + blocks.0.map_or(0, |x| x.size);
                self.clear_range(alloc_start, blocks.1 - alloc_start);

                let mut replacement = frame.next;

                if blocks.0.is_some() || blocks.2.is_some() {
                    let first_next = if blocks.2.is_some() {
                        Some(unsafe { NonNull::new_unchecked(blocks.1 as *mut FreeBlock) })
                    } else {
                        frame.next
                    };

                    if let Some(end_block) = blocks.2 {
                        replacement = Some(self.write_free_block(VirtAddr::new(blocks.1 as u64), end_block.size, frame.next));
                    }

                    if let Some(start_block) = blocks.0 {
                        replacement = Some(self.write_free_block(VirtAddr::new(frame_start as u64), start_block.size, first_next));
                    }
                }

                unsafe {
                    *link = replacement;
                }

                return Ok(alloc_start as *mut u8);
            } 

            link = unsafe {
                &raw mut (*current.as_ptr()).next
            };
        }
    }

    pub fn deallocate(&self, address: *mut u8, layout: Layout) {
        
    }
}