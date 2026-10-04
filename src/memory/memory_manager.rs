use bootloader::BootInfo;
use spin::{Mutex, MutexGuard, Once};
use x86_64::{PhysAddr, VirtAddr, registers::control::Cr3};

use crate::memory::{page_table::PageTable, physical_memory::PhysicalMemoryManager};

static MEMORY: Once<Mutex<MemoryManager>> = Once::new();

pub struct MemoryManager {
    physical_memory_manager: PhysicalMemoryManager,
    physical_memory_offset: VirtAddr,
    pml4: VirtAddr,

}

impl MemoryManager {
    pub fn new(boot_info: &'static BootInfo) -> Self {
        Self {
            physical_memory_manager: PhysicalMemoryManager::new(boot_info),
            physical_memory_offset: VirtAddr::new(boot_info.physical_memory_offset),
            pml4: Self::cache_active_level_4_table(VirtAddr::new(boot_info.physical_memory_offset)),
        }
    }

    pub fn cache_active_level_4_table(physical_memory_offset: VirtAddr) -> VirtAddr {
        let (level_4_table_frame, _) = Cr3::read();
        let phys = level_4_table_frame.start_address();
        let virt = physical_memory_offset + phys.as_u64();
        
        return virt;
    }
}

pub fn init(boot_info: &'static BootInfo) {
    MEMORY.call_once(|| {
        Mutex::new(MemoryManager::new(boot_info))
    });
}

pub fn memory_manager() -> MutexGuard<'static, MemoryManager> {
    MEMORY
        .wait()
        .expect("memory subsystem not initialized")
        .lock()
}