use bootloader::BootInfo;
use lazy_static::lazy_static;
use spin::{Mutex, MutexGuard, Once};
use x86_64::{PhysAddr, VirtAddr, instructions::tlb, registers::control::Cr3, structures::paging::page_table::PageTableLevel};

use crate::memory::{page_table::{PAGE_TABLE_ENTRIES, PHYSICAL_ADDRESS_MASK, PageTable, PageTableEntry, PageTableFlags}, physical_memory::{MapError, PhysicalMemoryManager}};

static MEMORY: Once<Mutex<MemoryManager>> = Once::new();

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TranslateError {
    NotMapped,
    HugePageEncountered
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UnMapError {
    NotMapped,
    NotOwned,
    HugePageEncountered
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ReMapError {
    AlreadyMapped,
    NotMapped,
    NotOwned,
    HugePageEncountered
}

pub struct MemoryManager {
    physical_memory_manager: PhysicalMemoryManager,
    physical_memory_offset: VirtAddr,
    l1tf_poison_mask: u64,
    pml4: PhysAddr,

}

impl MemoryManager {
    pub fn new(boot_info: &'static BootInfo) -> Self {
        let physical_memory_manager = PhysicalMemoryManager::new(boot_info);
        let l1tf_poison_mask = !((1u64 << (physical_memory_manager.max_physical_address_bits - 1)) - 1);

        Self {
            physical_memory_manager: physical_memory_manager,
            physical_memory_offset: VirtAddr::new(boot_info.physical_memory_offset),
            l1tf_poison_mask: l1tf_poison_mask,
            pml4: Self::cache_active_level_4_table(),
        }
    }

    pub fn cache_active_level_4_table() -> PhysAddr {
        let (level_4_table_frame, _) = Cr3::read();
        let phys = level_4_table_frame.start_address();
        
        return phys;
    }

    fn create_page_table(&mut self) -> Result<PhysAddr, MapError> {
        let allocation_address = self.physical_memory_manager.allocate_frame()?;

        unsafe { 
            (&mut *self.page_table_ptr(allocation_address)).zero();
        }

        return Ok(allocation_address);
    }

    fn page_table_ptr(&self, physical_address: PhysAddr) -> *mut PageTable {
        return self.physical_memory_manager.mut_ptr_at::<PageTable>(physical_address);
    }

    fn page_table_entry_ptr(&self, table_physical_address: PhysAddr, virtual_address: VirtAddr, page_table_level: PageTableLevel) -> *mut PageTableEntry {
        let page_table_index = usize::from(virtual_address.page_table_index(page_table_level));
        let page_table = self.page_table_ptr(table_physical_address);

        return unsafe { (*page_table).entries.as_mut_ptr().add(page_table_index) };
    }

    fn get_next_page_table(&self, table_physical_address: PhysAddr, virtual_address: VirtAddr, page_table_level: PageTableLevel) -> Result<PhysAddr, TranslateError> {

        let next_page_table_entry = unsafe { & *self.page_table_entry_ptr(table_physical_address, virtual_address, page_table_level) };        

        let page_table_flags = next_page_table_entry.flags();

        if !page_table_flags.contains(PageTableFlags::PRESENT) {
            return Err(TranslateError::NotMapped);
        }

        if page_table_flags.contains(PageTableFlags::HUGE_PAGE) {
            return Err(TranslateError::HugePageEncountered);
        }

        return Ok(next_page_table_entry.physical_frame())
    }

    fn get_or_create_page_entry(&mut self, table_physical_address: PhysAddr, virtual_address: VirtAddr, page_table_level: PageTableLevel) -> Result<PhysAddr, MapError> {
        let next_page_table_address = self.get_next_page_table(table_physical_address, virtual_address, page_table_level);

        match next_page_table_address {
            Ok(next_page_table_address) => Ok(next_page_table_address),

            Err(TranslateError::NotMapped) => {
                let page_table_physical_address = self.create_page_table()?;

                let page_table_entry = unsafe { &mut *self.page_table_entry_ptr(table_physical_address, virtual_address, page_table_level) };

                *page_table_entry = PageTableEntry::new(page_table_physical_address, PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE);

                Ok(page_table_physical_address)
            },

            Err(TranslateError::HugePageEncountered) => Err(MapError::HugePageEncountered)
        }
    }

    pub fn allocate_page(&mut self, virtual_address: VirtAddr, flags: PageTableFlags) -> Result<(), MapError> {
        let page_table_level_3_address = self.get_or_create_page_entry(self.pml4, virtual_address, PageTableLevel::Four)?;

        let page_table_level_2_address = self.get_or_create_page_entry(page_table_level_3_address, virtual_address, PageTableLevel::Three)?;

        let page_table_level_1_address = self.get_or_create_page_entry(page_table_level_2_address, virtual_address, PageTableLevel::Two)?;

       
        let page_table_entry = unsafe { &mut *self.page_table_entry_ptr(page_table_level_1_address, virtual_address, PageTableLevel::One) };
        let page_table_entry_flags = page_table_entry.flags();

        if page_table_entry_flags.contains(PageTableFlags::PRESENT) {
            return Err(MapError::AlreadyMapped);
        }

        if page_table_entry_flags.contains(PageTableFlags::OWNED) {
            return Err(MapError::AlreadyOwned);
        }

        let page_physical_address = self.physical_memory_manager.allocate_frame()?;

        self.physical_memory_manager.zero_frame(page_physical_address);

        *page_table_entry = PageTableEntry::new(page_physical_address, flags | PageTableFlags::PRESENT | PageTableFlags::OWNED);

        tlb::flush(virtual_address);

        return Ok(());
    }

    pub fn page_table_level_1_address(&self, virtual_address: VirtAddr) -> Result<PhysAddr, TranslateError> {
        let page_table_level_3_address = self.get_next_page_table(self.pml4, virtual_address, PageTableLevel::Four)?;
        let page_table_level_2_address = self.get_next_page_table(page_table_level_3_address, virtual_address, PageTableLevel::Three)?;
        return self.get_next_page_table(page_table_level_2_address, virtual_address, PageTableLevel::Two);
    }

    pub fn translate_frame(&self, virtual_address: VirtAddr) -> Result<PhysAddr, TranslateError> {
        let page_table_level_1_address = self.page_table_level_1_address(virtual_address)?;

        let page_table_level_1_entry = unsafe { & *self.page_table_entry_ptr(page_table_level_1_address, virtual_address, PageTableLevel::One) };
        if !page_table_level_1_entry.flags().contains(PageTableFlags::PRESENT) {
            return Err(TranslateError::NotMapped);
        }

        return Ok(page_table_level_1_entry.physical_frame());
    }

    pub fn translate_address(&self, virtual_address: VirtAddr) -> Result<PhysAddr, TranslateError> {
        let physical_frame = self.translate_frame(virtual_address)?;

        return Ok(physical_frame + u64::from(virtual_address.page_offset()));
    }

    pub fn map_existing_frame(&mut self, virtual_address: VirtAddr) -> Result<(), ReMapError> {
        let page_table_level_1_address = self.page_table_level_1_address(virtual_address);

        let page_table_level_1_entry = match page_table_level_1_address {
            Ok(page_table_level_1_address) => unsafe { &mut *self.page_table_entry_ptr(page_table_level_1_address, virtual_address, PageTableLevel::One) },

            Err(TranslateError::NotMapped) => { return Err(ReMapError::NotMapped); },
            Err(TranslateError::HugePageEncountered) => { return Err(ReMapError::HugePageEncountered); }
        };

        let mut page_table_level_1_entry_flags = page_table_level_1_entry.flags();
        if !page_table_level_1_entry_flags.contains(PageTableFlags::OWNED) {
            return Err(ReMapError::NotOwned);
        }

        if page_table_level_1_entry_flags.contains(PageTableFlags::PRESENT) {
            return Err(ReMapError::AlreadyMapped);
        }

        page_table_level_1_entry_flags.insert(PageTableFlags::PRESENT);

        *page_table_level_1_entry = PageTableEntry::new(
            // Reverse L1TF mitigation through posioning
            PhysAddr::new((page_table_level_1_entry.physical_frame().as_u64() & !self.l1tf_poison_mask) & PHYSICAL_ADDRESS_MASK),
            page_table_level_1_entry_flags
        );

        tlb::flush(virtual_address);

        return Ok(());
    }

    pub fn unmap_page(&mut self, virtual_address: VirtAddr) -> Result<PageTableEntry, UnMapError> {
        return self.unmap_page_internal(virtual_address, false);
    }

    fn unmap_page_internal(&mut self, virtual_address: VirtAddr, clear_entry: bool) -> Result<PageTableEntry, UnMapError> {
        let page_table_level_1_address = self.page_table_level_1_address(virtual_address);

        let page_table_level_1_entry = match page_table_level_1_address {
            Ok(page_table_level_1_address) => unsafe { &mut *self.page_table_entry_ptr(page_table_level_1_address, virtual_address, PageTableLevel::One) },

            Err(TranslateError::NotMapped) => { return Err(UnMapError::NotMapped); }
            Err(TranslateError::HugePageEncountered) => { return Err(UnMapError::HugePageEncountered); }
        };

        let mut page_table_level_1_entry_flags = page_table_level_1_entry.flags();

        let mut page_table_entry_copy = page_table_level_1_entry.clone();

        if !page_table_level_1_entry_flags.contains(PageTableFlags::OWNED) {
            return Err(UnMapError::NotOwned);
        }

        if !page_table_level_1_entry_flags.contains(PageTableFlags::PRESENT) {
            if !clear_entry {
                return Err(UnMapError::NotMapped);
            }

            // Reverse L1TF mitigation through posioning
            page_table_entry_copy.set_physical_frame(PhysAddr::new(page_table_entry_copy.physical_frame().as_u64() & !self.l1tf_poison_mask));
        }

        

        if clear_entry {
            *page_table_level_1_entry = PageTableEntry::ZERO;
        } else {
            page_table_level_1_entry_flags.remove(PageTableFlags::PRESENT);

            *page_table_level_1_entry = PageTableEntry::new(
                // Mitigate L1TF through posioning
                PhysAddr::new((page_table_entry_copy.physical_frame().as_u64() | self.l1tf_poison_mask) & PHYSICAL_ADDRESS_MASK),
                page_table_level_1_entry_flags
            );
        }

        tlb::flush(virtual_address);

        return Ok(page_table_entry_copy);
    }

    pub fn deallocate_page(&mut self, virtual_address: VirtAddr) -> Result<(), UnMapError> {
        let page_entry = self.unmap_page_internal(virtual_address, true)?;

        self.physical_memory_manager.free_frame(page_entry.physical_frame());

        return Ok(())

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