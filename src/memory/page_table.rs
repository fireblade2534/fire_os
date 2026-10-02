
const PAGE_TABLE_ENTRIES: usize = 512;

#[derive(Clone)]
#[repr(transparent)]
pub struct PageTableEntry {
    entry: u64,
}

#[derive(Clone)]
#[repr(C)]
pub struct PageTable {
    pub entries: [PageTableEntry; PAGE_TABLE_ENTRIES],
}

