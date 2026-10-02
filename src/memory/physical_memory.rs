use core::cmp::min;

use bootloader::{BootInfo, bootinfo::MemoryRegionType};
use x86_64::PhysAddr;


const MAX_PHYSICAL_FRAMES: usize = 262144;
const BITMAP_WORDS: usize = MAX_PHYSICAL_FRAMES.div_ceil(64);

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

#[repr(C)]
pub struct PhysicalMemoryManager {
    occupancy_mask: [u64; BITMAP_WORDS]
}

impl PhysicalMemoryManager {
    pub fn new(boot_info: &'static BootInfo) -> Self {
        let mut occupancy_mask = [u64::MAX; BITMAP_WORDS];

        for page_frame in boot_info.memory_map.iter() {
            if page_frame.range.start_frame_number as usize >= MAX_PHYSICAL_FRAMES {
                continue;
            }

            match page_frame.region_type {
                MemoryRegionType::Usable => {
                    let end_frame_number = min(MAX_PHYSICAL_FRAMES as u64, page_frame.range.end_frame_number);

                    clear_bits(&mut occupancy_mask, page_frame.range.start_frame_number, end_frame_number);
                },

                _ => {}
            }
        }

        Self {
            occupancy_mask: occupancy_mask,
        }

    }

    pub fn allocate_frame(&mut self) -> Option<PhysAddr> {
        for word_index in 0..self.occupancy_mask.len() {
            let word_mask = !self.occupancy_mask[word_index];

            if word_mask != 0 {
                let raw_index = word_mask.trailing_zeros() as u64;

                self.occupancy_mask[word_index] |= 1u64 << raw_index;

                return Some(PhysAddr::new((raw_index + (word_index as u64) * 64) * 4096));
            }
        }

        return None;
    }
}