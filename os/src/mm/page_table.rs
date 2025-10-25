//! Implementation of [`PageTableEntry`] and [`PageTable`].
use super::{frame_alloc, FrameTracker, PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use bitflags::*;
// 仅当 V(Valid) 位为 1 时，页表项才是合法的；
// R/W/X 分别控制索引到这个页表项的对应虚拟页面是否允许读/写/取指；
// U 控制索引到这个页表项的对应虚拟页面是否在 CPU 处于 U 特权级的情况下是否被允许访问；
// G 我们不理会；
// A(Accessed) 记录自从页表项上的这一位被清零之后，页表项的对应虚拟页面是否被访问过；
// D(Dirty) 则记录自从页表项上的这一位被清零之后，页表项的对应虚拟页表是否被修改过。
bitflags! {
    /// page table entry flags
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
/// page table entry structure
pub struct PageTableEntry {
    /// bits of page table entry
    pub bits: usize,
}

impl PageTableEntry {
    /// Create a new page table entry
    /// 根据物理页号和页表项标志位创建一个页表项
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        PageTableEntry {
            bits: ppn.0 << 10 | flags.bits as usize,
        }
    }
    /// Create an empty page table entry
    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    /// Get the physical page number from the page table entry
    pub fn ppn(&self) -> PhysPageNum {
        (self.bits >> 10 & ((1usize << 44) - 1)).into()
    }
    /// Get the flags from the page table entry
    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits(self.bits as u8).unwrap()
    }
    /// The page pointered by page table entry is valid?
    pub fn is_valid(&self) -> bool {
        (self.flags() & PTEFlags::V) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is readable?
    pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::R) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is writable?
    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is executable?
    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::X) != PTEFlags::empty()
    }
    /// 用户态可访问
    pub fn user_accessible(&self)->bool{
        (self.flags() & PTEFlags::U) != PTEFlags::empty()
    }
}

/// page table structure
/// SV39 多级页表是以节点为单位进行管理的。
/// 每个节点恰好存储在一个物理页帧中，它的位置可以用一个物理页号来表示。

/// 每个应用的地址空间都对应一个不同的多级页表，
/// 这也就意味这不同页表的起始地址（即页表根节点的地址）是不一样的。 
/// 因此 PageTable 要保存它根节点的物理页号 root_ppn 作为页表唯一的区分标志。
/// 此外， 向量 frames 以 FrameTracker 的形式保存了页表所有的节点（包括根节点）
/// 所在的物理页帧。这与物理页帧管理模块 的测试程序是一个思路，即将这些 FrameTracker 
/// 的生命周期进一步绑定到 PageTable 下面。当 PageTable 生命周期结束后，
/// 向量 frames 里面的那些 FrameTracker 也会被回收，
/// 也就意味着存放多级页表节点的那些物理页帧 被回收了。
pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<FrameTracker>,
}

/// Assume that it won't oom when creating/mapping.
/// 当我们通过 new 方法新建一个 PageTable 的时候，
/// 它只需有一个根节点。为此我们需要分配一个物理页帧 FrameTracker 
/// 并挂在向量 frames 下，然后更新根节点的物理页号 root_ppn 。
impl PageTable {
    /// Create a new page table
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();
        PageTable {
            root_ppn: frame.ppn,
            frames: vec![frame],
        }
    }
    /// Temporarily used to get arguments from user space.
    /// 从SATP（Supervisor Address Translation and Protection）寄存器的值创建一个 PageTable 实例。在RISC-V架构中，SATP寄存器用于控制地址转换，其格式如下：
    // 高4位：模式位（MODE），用于指定地址转换模式
    // 低44位：页表根节点的物理页号（PPN）
    pub fn from_token(satp: usize) -> Self {
        Self {
            // 提取SATP寄存器中的PPN（物理页号）部分
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }
    /// Find PageTableEntry by VirtPageNum, create a frame for a 4KB page table if not exist
    /// 在多级页表找到一个虚拟页号对应的页表项的可变引用方便后续的读写。
    /// 如果在 遍历的过程中发现有节点尚未创建则会新建一个节点
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            // 如果是第三级（叶子级）页表（i==2），将当前页表项设为结果并跳出循环
            if i == 2 {
                result = Some(pte);
                break;
            }
            // 如果当前页表项无效，分配一个新的物理页帧
            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                // 将新页表项的物理页帧加入页表的帧追踪器向量中
                self.frames.push(frame);
            }
            // 更新 ppn 为当前页表项指向的物理页号，用于下一级查找
            ppn = pte.ppn();
        }
        result
    }
    /// Find PageTableEntry by VirtPageNum
    fn find_pte(&self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        result
    }
    /// set the map between virtual page number and physical page number
    /// 多级页表并不是被创建出来之后就不再变化的，
    /// 为了 MMU 能够通过地址转换正确找到应用地址空间中的数据实际被内核放在内存中位置，
    /// 操作系统需要动态维护一个虚拟页号到页表项的映射，支持插入/删除键值对
    #[allow(unused)]
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        assert!(!pte.is_valid(), "vpn {:?} is mapped before mapping", vpn);
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }
    /// remove the map between virtual page number and physical page number
    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmapping", vpn);
        *pte = PageTableEntry::empty();
    }
    /// get the page table entry from the virtual page number
    /// 如果能够找到页表项，那么它会将页表项拷贝一份并返回
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }
    /// get the physical address from the virtual address
    pub fn translate_va(&self, va: VirtAddr) -> Option<PhysAddr> {
        self.find_pte(va.clone().floor()).map(|pte| {
            //println!("translate_va:va = {:?}", va);
            let aligned_pa: PhysAddr = pte.ppn().into();
            //println!("translate_va:pa_align = {:?}", aligned_pa);
            let offset = va.page_offset();
            let aligned_pa_usize: usize = aligned_pa.into();
            (aligned_pa_usize + offset).into()
        })
    }
    /// get the token from the page table
    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
}

/// Translate&Copy a ptr[u8] array with LENGTH len to a mutable u8 Vec through page table
pub fn translated_byte_buffer(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();
        let ppn = page_table.translate(vpn).unwrap().ppn();
        vpn.step();
        let mut end_va: VirtAddr = vpn.into();
        end_va = end_va.min(VirtAddr::from(end));
        if end_va.page_offset() == 0 {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.into();
    }
    v
}

/// Translate&Copy a ptr[u8] array end with `\0` to a `String` Vec through page table
pub fn translated_str(token: usize, ptr: *const u8) -> String {
    let page_table = PageTable::from_token(token);
    let mut string = String::new();
    let mut va = ptr as usize;
    loop {
        let ch: u8 = *(page_table
            .translate_va(VirtAddr::from(va))
            .unwrap()
            .get_mut());
        if ch == 0 {
            break;
        } else {
            string.push(ch as char);
            va += 1;
        }
    }
    string
}
/// Translate a ptr[u8] array through page table and return a mutable reference of T
pub fn translated_refmut<T>(token: usize, ptr: *mut T) -> &'static mut T {
    //trace!("into translated_refmut!");
    let page_table = PageTable::from_token(token);
    let va = ptr as usize;
    //trace!("translated_refmut: before translate_va");
    page_table
        .translate_va(VirtAddr::from(va))
        .unwrap()
        .get_mut()
}
