use core::{alloc::Layout, ptr::{NonNull, slice_from_raw_parts_mut}};

use x86_64::VirtAddr;
use lazy_static::lazy_static;

use crate::memory::{memory_manager::{MemoryManager, PAGE_SIZE, TranslateError, memory_manager}, page_table::PageTableFlags};

pub const HEAP_START: VirtAddr = VirtAddr::new(0x_4444_4444_0000);
pub const HEAP_SIZE: usize = 100 * 1024; 

lazy_static! {
    static ref HEAP_FLAGS: PageTableFlags = PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
}
fn align_up(value: usize, align: usize) -> usize {
    assert_eq!(align & (align - 1), 0);

    return (value + (align - 1)) & !(align - 1)
}

pub struct Heap {
    start_address: VirtAddr,
    start_block: Option<NonNull<FreeBlock>>,

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

        memory.allocate_page(self.start_address, *HEAP_FLAGS).expect("Failed to allocate inital page for the heap");

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

    fn dynamicly_allocate(&self, address: u64, flags: PageTableFlags) -> Result<(), ()> {
        let mut memory_manager = memory_manager();

        let virtual_address = VirtAddr::new(address);
        match memory_manager.translate_frame(virtual_address) {
            Ok(_) => Ok(()),

            Err(TranslateError::NotMapped) => {
                memory_manager.allocate_page(virtual_address, flags).map_err(|_| ())
            },

            Err(_) => Err(()),
        }
    }

    fn dynamicly_allocate_range(&self, start_address: u64, end_address: u64, flags: PageTableFlags) -> Result<(), ()> {
        let start_frame = start_address / PAGE_SIZE;
        let end_frame = (end_address - 1) / PAGE_SIZE;
        
        for frame in start_frame..=end_frame {
            self.dynamicly_allocate(frame * PAGE_SIZE, flags)?;
        }

        return Ok(());
    }

    fn dynamicly_deallocate(&self, address: u64) -> Result<(), ()> {
        let mut memory_manager = memory_manager();

        let virtual_address = VirtAddr::new(address);
        match memory_manager.translate_frame(virtual_address) {
            Ok(_) => {
                memory_manager.deallocate_page(virtual_address).map_err(|_| ())
            },

            Err(_) => Err(()),
        }
    }

    fn dynamicly_deallocate_range(&self, start_address: u64, end_address: u64) -> Result<(), ()> {
        let start_frame = start_address / PAGE_SIZE;
        let end_frame = (end_address - 1) / PAGE_SIZE;
        
        for frame in start_frame..=end_frame {
            self.dynamicly_deallocate(frame * PAGE_SIZE)?;
        }

        return Ok(());
    }

    pub fn allocate(&mut self, layout: Layout) -> Result<*mut u8, ()> {
        let frame_object_size = size_of::<FreeBlock>() as u64;

        let mut link: *mut Option<NonNull<FreeBlock>> = &raw mut self.start_block;
        unsafe {
            while let Some(current) = *link {
                let frame = *current.as_ptr();
                let frame_start = current.as_ptr() as usize;

                let blocks = self.check_layout_fit(frame_start, &frame, layout);

                if let Some(blocks) = blocks {
                    let alloc_start = frame_start + blocks.0.map_or(0, |x| x.size);

                    self.dynamicly_allocate_range(alloc_start as u64, blocks.1 as u64, *HEAP_FLAGS)?;

                    let mut replacement = frame.next;

                    if blocks.0.is_some() || blocks.2.is_some() {
                        let first_next = if blocks.2.is_some() {
                            Some(NonNull::new_unchecked(blocks.1 as *mut FreeBlock))
                        } else {
                            frame.next
                        };

                        if let Some(end_block) = blocks.2 {
                            self.dynamicly_allocate_range(blocks.1 as u64, blocks.1 as u64 + frame_object_size, *HEAP_FLAGS)?;
                            replacement = Some(self.write_free_block(VirtAddr::new(blocks.1 as u64), end_block.size, frame.next));
                        }

                        if let Some(start_block) = blocks.0 {
                            self.dynamicly_allocate_range(frame_start as u64, frame_start as u64 + frame_object_size, *HEAP_FLAGS)?;
                            replacement = Some(self.write_free_block(VirtAddr::new(frame_start as u64), start_block.size, first_next));
                        }
                    }

                    *link = replacement;
                    
                    return Ok(alloc_start as *mut u8);
                } 

                link = &raw mut (*current.as_ptr()).next;
            }
        }
        return Err(());
    }

    pub fn deallocate(&mut self, address: *mut u8, layout: Layout) -> Result<(), ()> {
        let frame_object_size = size_of::<FreeBlock>();
        let frame_object_align = align_of::<FreeBlock>();

        let allocation_start_address= address as usize;

        
        let allocation_end_address = align_up(allocation_start_address + layout.size().max(frame_object_size), frame_object_align);
        let allocation_size = allocation_end_address - allocation_start_address;

        let mut link: *mut Option<NonNull<FreeBlock>> = &raw mut self.start_block;
        let mut previous: Option<NonNull<FreeBlock>> = None;
        unsafe {
            while let Some(current) = *link {
                if (current.as_ptr() as usize) >= allocation_start_address {
                    break;
                }
                previous = Some(current);

                link = &raw mut (*current.as_ptr()).next;
            }
        }

        let next = unsafe { *link };

        unsafe {
            if let Some(previous_block) = previous {
                let previous_block_size = (*previous_block.as_ptr()).size;
                let previous_block_end = previous_block.as_ptr() as usize + previous_block_size;
                assert!(
                    previous_block_end <= allocation_start_address,
                    "Heap deallocation overlaps previous free block"
                );
            }

            if let Some(next_block) = next {
                let next_block_start = next_block.as_ptr() as usize;
                assert!(
                    allocation_end_address <= next_block_start,
                    "Heap deallocation overlaps next free block"
                );
            }
        }

        let new_block = self.write_free_block(
            VirtAddr::new(allocation_start_address as u64),
            allocation_size,
            next,
        );

        unsafe {
            *link = Some(new_block);

            if let Some(next_block) = next {
                if next_block.as_ptr() as usize == allocation_end_address {
                    (*new_block.as_ptr()).next = (*next_block.as_ptr()).next;
                    (*new_block.as_ptr()).size += (*next_block.as_ptr()).size;
                }
            }

            if let Some(previous_block) = previous {
                let previous_block_size = (*previous_block.as_ptr()).size;
                if previous_block.as_ptr() as usize + previous_block_size == allocation_start_address {
                    (*previous_block.as_ptr()).next = (*new_block.as_ptr()).next;
                    (*previous_block.as_ptr()).size += (*new_block.as_ptr()).size;
                }
            }
        }

        // Basic best effort dynamic reclamation
        let _ = self.dynamicly_deallocate_range(allocation_start_address as u64 + frame_object_size as u64 + PAGE_SIZE, allocation_end_address as u64 - PAGE_SIZE);

        return Ok(());
    }
}