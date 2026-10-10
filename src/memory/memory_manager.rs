use bootloader::BootInfo;
use lazy_static::lazy_static;
use spin::{Mutex, MutexGuard, Once};
use x86_64::{PhysAddr, VirtAddr, instructions::tlb, registers::control::Cr3, structures::paging::PageTableIndex};

use crate::memory::{page_table::{PHYSICAL_ADDRESS_MASK, PageTable, PageTableEntry, PageTableFlags, PageTableLevel}, physical_memory::{MapError, PhysicalMemoryManager, TransactionMask}};

static MEMORY: Once<Mutex<MemoryManager>> = Once::new();

pub static PAGE_SIZE: u64 = 4096;

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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ActionError {
    NoneUsed,
    FrameUsed
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum MemoryAction {
    None,
    AllocateFrame(PhysAddr),
    FreeFrame(PhysAddr),
    CreatePageTable(PhysAddr),
    RemovePageTable(PhysAddr),
    ModifyPageTableEntry(PhysAddr, PageTableIndex, PageTableEntry)
}

struct MemoryActionPlan {
    plan: [MemoryAction; 32],
    plan_length: u8,

    physical_memory_excluded: [TransactionMask; 16],
    physical_memory_excluded_index: u8,

    physical_memory_included: [TransactionMask; 16],
    physical_memory_included_index: u8,
}

impl MemoryActionPlan {
    pub fn new() -> Self {
        Self {
            plan: [MemoryAction::None; 32],
            plan_length: 0,
            physical_memory_excluded: [TransactionMask::default(); 16],
            physical_memory_excluded_index: 0,
            physical_memory_included: [TransactionMask::default(); 16],
            physical_memory_included_index: 0,
        }
    }

    fn add_transaction(array: &mut [TransactionMask], index: &mut u8, frame: PhysAddr) {
        let frame_number = frame.as_u64() / PAGE_SIZE;

        let frame_index = frame_number / 64;
        let word_index = frame_number & 63;
        for transaction in &mut array[0..*index as usize] {
            if transaction.word == frame_index {

                transaction.mask |= 1u64 << word_index;
                return;
            }
        }

        array[*index as usize] = TransactionMask {
            word: frame_index,
            mask: 1u64 << word_index,
        };

        *index += 1;
    }

    pub fn physical_memory_excluded(&self) -> &[TransactionMask] {
        return &self.physical_memory_excluded[0..self.physical_memory_excluded_index as usize];
    }

    pub fn physical_memory_included(&self) -> &[TransactionMask] {
        return &self.physical_memory_included[0..self.physical_memory_included_index as usize];
    }

    pub fn add_plan_item(&mut self, action: MemoryAction) {
        let max_entries = self.plan.len();
        if self.plan_length as usize >= max_entries {
            panic!("Memory action plan has more then {max_entries} actions");
        }

        self.plan[self.plan_length as usize] = action;
        self.plan_length += 1;

        match action {
            MemoryAction::AllocateFrame(frame) | MemoryAction::CreatePageTable(frame) => {
                MemoryActionPlan::add_transaction(&mut self.physical_memory_excluded, &mut self.physical_memory_excluded_index, frame);
            },
            MemoryAction::FreeFrame(frame) | MemoryAction::RemovePageTable(frame) => {
                // For now don't include freeing frames as part of the transactions as it complicates verification
                //MemoryActionPlan::add_transaction(&mut self.physical_memory_included, &mut self.physical_memory_included_index, frame);
            },

            _ => {}
        }

        return;
    }

    pub fn iter(&self) -> core::slice::Iter<'_, MemoryAction> {
        self.plan[0..self.plan_length as usize].iter()
    }
}


fn get_page_index(address: VirtAddr, level: PageTableLevel) -> PageTableIndex {
    match level {
        PageTableLevel::One => address.p1_index(),
        PageTableLevel::Two => address.p2_index(),
        PageTableLevel::Three => address.p3_index(),
        PageTableLevel::Four => address.p4_index()
    }
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

    fn page_table_ptr(&self, physical_address: PhysAddr) -> *mut PageTable {
        return self.physical_memory_manager.mut_ptr_at::<PageTable>(physical_address);
    }

    fn page_table_entry_index_ptr(&self, table_physical_address: PhysAddr, index: PageTableIndex) -> *mut PageTableEntry {
        let page_table_index = usize::from(index);
        let page_table = self.page_table_ptr(table_physical_address);

        return unsafe { (*page_table).entries.as_mut_ptr().add(page_table_index) };
    }

    fn page_table_entry_ptr(&self, table_physical_address: PhysAddr, virtual_address: VirtAddr, page_table_level: PageTableLevel) -> *mut PageTableEntry {
        let page_table_index = usize::from(get_page_index(virtual_address, page_table_level));
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

    fn execute_action(&mut self, action: &MemoryAction) {
        match action {
            MemoryAction::AllocateFrame(frame) | MemoryAction::CreatePageTable(frame) => {
                self.physical_memory_manager.allocate_frame_at_addr(*frame);

                self.physical_memory_manager.zero_frame(*frame);
            },
            MemoryAction::FreeFrame(frame) | MemoryAction::RemovePageTable(frame) => {
                self.physical_memory_manager.free_frame(*frame);
            },
            MemoryAction::ModifyPageTableEntry(page_table, page_table_index, page_table_entry) => {
                let entry_ptr = self.page_table_entry_index_ptr(
                    *page_table,
                    *page_table_index,
                );

                let entry = unsafe { &mut *entry_ptr };

                *entry = *page_table_entry;
            },
            MemoryAction::None => {},
        }
    }

    fn execute_action_plan(&mut self, virtual_address: VirtAddr, plan: &MemoryActionPlan) {

        // Stage 1: Allocate frames
        for action in plan.iter() {
            if matches!(action, MemoryAction::AllocateFrame(..) | MemoryAction::CreatePageTable(..)) {
                self.execute_action(action);
            }
        }

        // Stage 2: Modify page table entries
        for action in plan.iter().rev() {
            if matches!(action, MemoryAction::ModifyPageTableEntry(..)) {
                self.execute_action(action);
            }
        }

        tlb::flush(virtual_address);

        // Stage 3: Deallocate frames
        for action in plan.iter() {
            if matches!(action, MemoryAction::FreeFrame(..) | MemoryAction::RemovePageTable(..)) {
                self.execute_action(action);
            }
        }
    }

    pub fn allocate_page(&mut self, virtual_address: VirtAddr, flags: PageTableFlags) -> Result<(), MapError> {
        let parent_flags = flags & (PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE);

        let mut plan = MemoryActionPlan::new();

        let mut leaf_table_is_new = true;
        let mut table_address = self.pml4;
        for level in (2..=4).rev() {
            let page_table_level = PageTableLevel::from_index(level).unwrap();

            let next_page_table = self.get_next_page_table(table_address, virtual_address, page_table_level);

            match next_page_table {
                Ok(next_page_table) => {
                    let entry_ptr = self.page_table_entry_ptr(
                        table_address,
                        virtual_address,
                        page_table_level,
                    );

                    let entry = unsafe { *entry_ptr };
                    let old_flags = entry.flags();

                    let required_flags = old_flags | parent_flags;

                    if required_flags != old_flags {
                        let mut new_entry = entry;
                        new_entry.set_flags(required_flags);

                        plan.add_plan_item(
                            MemoryAction::ModifyPageTableEntry(
                                table_address,
                                get_page_index(virtual_address, page_table_level),
                                new_entry,
                            )
                        );
                    }

                    if level == 2 {
                        leaf_table_is_new = false;
                    }

                    table_address = next_page_table;
                },

                Err(TranslateError::NotMapped) => {
                    for new_level in (2..=level).rev() {
                        let new_table_address = self.physical_memory_manager.find_free_frame_transaction(plan.physical_memory_excluded(), plan.physical_memory_included())?;
                        
                        plan.add_plan_item(MemoryAction::CreatePageTable(new_table_address));

                        let page_table_index = get_page_index(virtual_address, PageTableLevel::from_index(new_level).unwrap());

                        let page_table_entry = PageTableEntry::new(new_table_address, parent_flags | PageTableFlags::PRESENT | PageTableFlags::OWNED);

                        plan.add_plan_item(MemoryAction::ModifyPageTableEntry(table_address, page_table_index, page_table_entry));

                        table_address = new_table_address;
                    }

                    break;
                },
                Err(TranslateError::HugePageEncountered) => {
                    return Err(MapError::HugePageEncountered);
                }
            }
        }

        if !leaf_table_is_new {
            let entry = unsafe {
                *self.page_table_entry_ptr(
                    table_address,
                    virtual_address,
                    PageTableLevel::One,
                )
            };

            let entry_flags = entry.flags();

            if entry_flags.contains(PageTableFlags::PRESENT) {
                return Err(MapError::AlreadyMapped);
            }

            if entry_flags.contains(PageTableFlags::OWNED) {
                return Err(MapError::AlreadyOwned);
            }
        }

        let frame_address = self.physical_memory_manager.find_free_frame_transaction(plan.physical_memory_excluded(), plan.physical_memory_included())?;

        plan.add_plan_item(MemoryAction::AllocateFrame(frame_address));

        let page_table_entry = PageTableEntry::new(frame_address, flags | PageTableFlags::PRESENT | PageTableFlags::OWNED);
        let page_table_index = get_page_index(virtual_address, PageTableLevel::One);
        plan.add_plan_item(MemoryAction::ModifyPageTableEntry(table_address, page_table_index, page_table_entry));

        self.execute_action_plan(virtual_address, &plan);

        return Ok(());
    }

    pub fn deallocate_page(&mut self, virtual_address: VirtAddr) -> Result<(), UnMapError> {
        let mut plan = MemoryActionPlan::new();

        let mut table_address = [PhysAddr::zero(), PhysAddr::zero(), PhysAddr::zero(), self.pml4];
        for level in (2..=4).rev() {
            let page_table_level = PageTableLevel::from_index(level).unwrap();

            let next_page_table = self.get_next_page_table(table_address[level as usize - 1], virtual_address, page_table_level);

            match next_page_table {
                Ok(next_page_table) => {
                    table_address[level as usize - 2] = next_page_table;
                },

                Err(TranslateError::NotMapped) => {
                    return Err(UnMapError::NotMapped)
                },

                Err(TranslateError::HugePageEncountered) => {
                    return Err(UnMapError::HugePageEncountered);
                }
            }
        }

        let entry = unsafe {
            *self.page_table_entry_ptr(
                table_address[0],
                virtual_address,
                PageTableLevel::One,
            )
        };

        let entry_flags = entry.flags();
        if !entry_flags.contains(PageTableFlags::PRESENT) && !entry_flags.contains(PageTableFlags::OWNED) {
            return Err(UnMapError::NotMapped);
        }

        if !entry_flags.contains(PageTableFlags::OWNED) {
            return Err(UnMapError::NotOwned);
        }
        
        let physical_frame = if entry_flags.contains(PageTableFlags::PRESENT) {
            entry.physical_frame()
        } else {
            // Reverse L1TF posioning mitigation 
            entry
                .unpoison(self.l1tf_poison_mask)
                .physical_frame()
        };

        plan.add_plan_item(MemoryAction::FreeFrame(physical_frame));

        let mut last_level = u8::MAX;
        for level in 1..=3 {
            let page_table = unsafe { & *self.page_table_ptr(table_address[level - 1]) };

            if page_table.count_occupied() != 1 {
                break;
            }

            let parent_entry_ptr = self.page_table_entry_ptr(
                table_address[level],
                virtual_address,
                PageTableLevel::from_index(level as u8 + 1).unwrap(),
            );

            let parent_entry = unsafe { *parent_entry_ptr };
            let parent_entry_flags = parent_entry.flags();

            if !parent_entry_flags.contains(PageTableFlags::OWNED) {
                break;
            }

            last_level = level as u8;
        }

        if last_level != u8::MAX {
            let page_table_index = get_page_index(virtual_address, PageTableLevel::from_index(last_level + 1).unwrap());
            plan.add_plan_item(MemoryAction::ModifyPageTableEntry(table_address[last_level as usize], page_table_index, PageTableEntry::ZERO));

            for level in 1..=last_level {
                plan.add_plan_item(MemoryAction::RemovePageTable(table_address[level as usize - 1]));
            }
        } else {
            let page_table_index = get_page_index(virtual_address, PageTableLevel::One);
            plan.add_plan_item(MemoryAction::ModifyPageTableEntry(table_address[0], page_table_index, PageTableEntry::ZERO));
        }

        self.execute_action_plan(virtual_address, &plan);

        return Ok(());
    }

    pub fn page_table_level_1_address(&self, virtual_address: VirtAddr) -> Result<PhysAddr, TranslateError> {
        let page_table_level_3_address = self.get_next_page_table(self.pml4, virtual_address, PageTableLevel::Four)?;
        let page_table_level_2_address = self.get_next_page_table(page_table_level_3_address, virtual_address, PageTableLevel::Three)?;
        return self.get_next_page_table(page_table_level_2_address, virtual_address, PageTableLevel::Two);
    }

    pub fn remap_page(&mut self, virtual_address: VirtAddr) -> Result<(), ReMapError> {
        let mut plan = MemoryActionPlan::new();

        let temp_table = self.page_table_level_1_address(virtual_address);

        let table_address = match temp_table {
            Err(TranslateError::HugePageEncountered) => { return Err(ReMapError::HugePageEncountered); },
            Err(TranslateError::NotMapped) => { return Err(ReMapError::NotMapped); },
            Ok(physical_address) => { physical_address }
        };

        let entry = unsafe {
            *self.page_table_entry_ptr(
                table_address,
                virtual_address,
                PageTableLevel::One,
            )
        };

        let entry_flags = entry.flags();

        if entry_flags.contains(PageTableFlags::PRESENT) {
            return Err(ReMapError::AlreadyMapped);
        }

        if !entry_flags.contains(PageTableFlags::OWNED) {
            return Err(ReMapError::NotOwned);
        }

        // Reverse L1TF posioning mitigation 
        let mut new_entry = entry.unpoison(self.l1tf_poison_mask);
        new_entry.set_flags(
            new_entry.flags() | PageTableFlags::PRESENT
        );

        let page_table_index = get_page_index(virtual_address, PageTableLevel::One);
        plan.add_plan_item(MemoryAction::ModifyPageTableEntry(table_address, page_table_index, new_entry));

        self.execute_action_plan(virtual_address, &plan);

        return Ok(());
    }

    pub fn unmap_page(&mut self, virtual_address: VirtAddr) -> Result<PageTableEntry, UnMapError> {
        let mut plan = MemoryActionPlan::new();

        let temp_table = self.page_table_level_1_address(virtual_address);

        let table_address = match temp_table {
            Err(TranslateError::HugePageEncountered) => { return Err(UnMapError::HugePageEncountered); },
            Err(TranslateError::NotMapped) => { return Err(UnMapError::NotMapped); },
            Ok(physical_address) => { physical_address }
        };

        let entry = unsafe {
            *self.page_table_entry_ptr(
                table_address,
                virtual_address,
                PageTableLevel::One,
            )
        };

        let entry_flags = entry.flags();

        if !entry_flags.contains(PageTableFlags::PRESENT) {
            return Err(UnMapError::NotMapped);
        }

        if !entry_flags.contains(PageTableFlags::OWNED) {
            return Err(UnMapError::NotOwned);
        }

        // Mitigate L1TF through posioning
        let mut new_entry = entry.poison(self.l1tf_poison_mask);
        new_entry.set_flags(
            new_entry.flags() & !PageTableFlags::PRESENT
        );

        let page_table_index = get_page_index(virtual_address, PageTableLevel::One);
        plan.add_plan_item(MemoryAction::ModifyPageTableEntry(table_address, page_table_index, new_entry));

        self.execute_action_plan(virtual_address, &plan);

        return Ok(entry);
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