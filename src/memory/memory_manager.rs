use bootloader::BootInfo;
use x86_64::{PhysAddr, VirtAddr, registers::control::Cr3};

use crate::memory::page_table::PageTable;

pub struct MemoryManager {
    physical_memory_offset: VirtAddr,
    pml4: *mut PageTable,

}

impl MemoryManager {
    pub fn new(boot_info: &'static BootInfo) -> Self {
        Self {
            physical_memory_offset: VirtAddr::new(boot_info.physical_memory_offset),
            pml4: Self::active_level_4_table(VirtAddr::new(boot_info.physical_memory_offset)),
        }
    }

    pub fn active_level_4_table(physical_memory_offset: VirtAddr) -> *mut PageTable {
        let (level_4_table_frame, _) = Cr3::read();
        let phys = level_4_table_frame.start_address();
        let virt = physical_memory_offset + phys.as_u64();
        
        return unsafe { *virt.as_mut_ptr() };
    }
}