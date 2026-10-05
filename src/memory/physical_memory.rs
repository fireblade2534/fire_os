use core::{arch::x86_64::__cpuid, cmp::min, slice};

use bootloader::{BootInfo, bootinfo::MemoryRegionType};
use x86_64::{PhysAddr, VirtAddr};

use crate::{print, println};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    AlreadyMapped,
    AlreadyOwned,
    OutOfPhysicalMemory,
    HugePageEncountered
}

fn max_physical_address_bits() -> u8 {
    // First check that extended leaf 0x8000_0008 exists.
    let max_extended = __cpuid(0x8000_0000).eax;

    assert!(
        max_extended >= 0x8000_0008,
        "CPU does not support CPUID leaf 0x80000008"
    );

    let result = __cpuid(0x8000_0008);

    (result.eax & 0xff) as u8
}

fn clear_bits(words: &mut [u64], start_bit: u64, end_bit: u64) {
    if start_bit >= end_bit {
        return;
    }

    let start_word = start_bit / 64;
    let end_word = (end_bit - 1) / 64;

    let start_word_bit = start_bit & 63;
    let end_word_bit = end_bit & 63;

    let start_mask = (1u64 << start_word_bit) - 1;
    let end_mask = if end_word_bit == 0 { 0 } else { u64::MAX << end_word_bit };

    if start_word == end_word {
        words[start_word as usize] &= start_mask | end_mask;
    } else {
        words[start_word as usize] &= start_mask;
        words[(start_word + 1) as usize..end_word as usize].fill(0);
        words[end_word as usize] &= end_mask;
    }
}

fn fill_bits(words: &mut [u64], start_bit: u64, end_bit: u64) {
    if start_bit >= end_bit {
        return;
    }

    let start_word = start_bit / 64;
    let end_word = (end_bit - 1) / 64;

    let start_word_bit = start_bit & 63;
    let end_word_bit = end_bit & 63;

    let start_mask = u64::MAX << start_word_bit;
    let end_mask = if end_word_bit == 0 { u64::MAX } else { (1u64 << end_word_bit) - 1};

    if start_word == end_word {
        words[start_word as usize] |= start_mask & end_mask;
    } else {
        words[start_word as usize] |= start_mask;
        words[(start_word + 1) as usize..end_word as usize].fill(u64::MAX);
        words[end_word as usize] |= end_mask;
    }
}

#[repr(C)]
pub struct PhysicalMemoryManager {
    max_usable_frame: u64,
    total_bitmap_items: u64,
    pub max_physical_address_bits: u8,
    occupancy_mask_ptr: VirtAddr,
    physical_memory_ptr: VirtAddr,
}

impl PhysicalMemoryManager {
    pub fn new(boot_info: &'static BootInfo) -> Self {

        let max_physical_address_bits = max_physical_address_bits();
        println!("CPU max physical address bits {max_physical_address_bits}");
        
        let mut max_usable_frame: u64 = 0;
        for page_frame in boot_info.memory_map.iter() {
            if page_frame.region_type == MemoryRegionType::Usable {
                if page_frame.range.end_frame_number > max_usable_frame {
                    max_usable_frame = page_frame.range.end_frame_number;
                }
            }
        }

        println!("Physical frame upper bound {max_usable_frame}");

        let total_bitmap_frames = max_usable_frame.div_ceil(32768);

        let mut best_range_score: u64 = u64::MAX;
        let mut best_range_start: u64 = 0;
        for page_frame in boot_info.memory_map.iter() {
            if page_frame.region_type == MemoryRegionType::Usable {
                let range_diff = page_frame.range.end_frame_number - page_frame.range.start_frame_number;

                if range_diff >= total_bitmap_frames {
                    let score = range_diff - total_bitmap_frames;

                    if score < best_range_score {
                        best_range_score = score;
                        best_range_start = page_frame.range.start_frame_number;
                    }
                }
            }
        }

        if best_range_score == u64::MAX {
            panic!("No useable contiguous {total_bitmap_frames} frame region for the occupancy mask found");
        }

        let mask_start_address = best_range_start * 4096;

        println!("Found useable contiguous {total_bitmap_frames} frame region for the occupancy mask");
        println!("  - Region start physical address: {mask_start_address:#x}");
        println!("  - Region ideal difference: {best_range_score}");

        let total_bitmap_items = max_usable_frame.div_ceil(64);

        let mut manager = Self {
            max_usable_frame,
            total_bitmap_items: total_bitmap_items,
            max_physical_address_bits: max_physical_address_bits,
            occupancy_mask_ptr: VirtAddr::new(boot_info.physical_memory_offset + mask_start_address),
            physical_memory_ptr: VirtAddr::new(boot_info.physical_memory_offset)
        };

        println!("Creating occupancy bitmap");
        let mask = manager.occupancy_mask();

        println!("Marking occupancy bitmap as default occupied");
        mask.fill(u64::MAX);
        
        println!("Marking useable frames");
        let mut useable_frames: u64 = 0;
        for page_frame in boot_info.memory_map.iter() {
            match page_frame.region_type {
                MemoryRegionType::Usable => {
                    useable_frames += page_frame.range.end_frame_number - page_frame.range.start_frame_number;
                    clear_bits(mask, page_frame.range.start_frame_number, page_frame.range.end_frame_number);
                },

                _ => {}
            }
        }
        println!("Marked {useable_frames} frames as useable");

        println!("Marking bitmap pages as occupied");
        fill_bits(mask, best_range_start, best_range_start + total_bitmap_frames);


        return manager;

    }

    pub fn allocate_frame(&mut self) -> Result<PhysAddr, MapError> {
        let mask = self.occupancy_mask();

        for word_index in 0..mask.len() {
            let word_mask = !mask[word_index as usize];

            if word_mask != 0 {
                let raw_index = word_mask.trailing_zeros() as u64;

                mask[word_index as usize] |= 1u64 << raw_index;

                return Ok(PhysAddr::new((raw_index + (word_index as u64) * 64) * 4096));
            }
        }

        return Err(MapError::OutOfPhysicalMemory);
    }

    pub fn free_frame(&mut self, frame: PhysAddr) {
        let mask = self.occupancy_mask();

        let frame_number = frame.as_u64() / 4096;

        let frame_index = frame_number / 64;
        let word_index = frame_number & 63;

        mask[frame_index as usize] &= !(1u64 << word_index);
    }

    fn occupancy_mask(&mut self) -> &mut [u64] {
        unsafe {
            slice::from_raw_parts_mut(
                self.occupancy_mask_ptr.as_mut_ptr::<u64>(),
                self.total_bitmap_items as usize,
            )
        }
    }

    pub fn ptr_at<T>(&self, address: PhysAddr) -> *const T {
        let offset = address.as_u64() as usize;

        let base = self.physical_memory_ptr.as_ptr::<u8>();

        let ptr = unsafe {
            base.add(offset).cast::<T>()
        };

        return ptr;
    }

    pub fn mut_ptr_at<T>(&self, address: PhysAddr) -> *mut T {
        let offset = address.as_u64() as usize;

        let base = self.physical_memory_ptr.as_mut_ptr::<u8>();

        let ptr = unsafe {
            base.add(offset).cast::<T>()
        };

        return ptr;
    }

    pub unsafe fn read_u64(&self, address: PhysAddr) -> u64 {
        unsafe {
            self.ptr_at::<u64>(address).read()
        }
    }

    pub unsafe fn write_u64(&self, address: PhysAddr, value: u64) {
        unsafe {
            self.mut_ptr_at::<u64>(address).write(value);
        }
    }

    pub fn zero_frame(&self, address: PhysAddr) {
        unsafe {
            core::ptr::write_bytes(
                self.mut_ptr_at::<u8>(address),
                0,
                4096,
            );
        }
    }
}

