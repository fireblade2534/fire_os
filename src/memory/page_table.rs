use x86_64::PhysAddr;


const PAGE_TABLE_ENTRIES: usize = 512;

#[derive(Clone)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub fn flags(self) -> u16 {
        return (self.0 & 0xFFF0_0000_0000_0000) as u16;
    }
    
    pub fn physical_frame(self) -> PhysAddr {
        return PhysAddr::new(self.0 & 0x000F_FFFF_FFFF_F000);
    }
}

#[derive(Clone)]
#[repr(C)]
pub struct PageTable {
    pub entries: [PageTableEntry; PAGE_TABLE_ENTRIES],
}