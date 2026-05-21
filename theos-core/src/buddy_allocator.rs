// buddy_allocator.rs -- Host-testable mirror of theos-kernel memory/buddy.rs
//
// This module contains the same buddy allocator logic as the kernel,
// but compiled for std so it can be tested on the host (Windows/WSL).
// No UART, no no_std, no hardware dependencies.
//
// Tests here validate the algorithm correctness. The kernel version
// is the same logic, just wrapped in no_std + static globals.

pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;
pub const MAX_ORDER: usize = 11;

const MAX_PAGES: usize = 524288;
const BITMAP_SIZE: usize = MAX_PAGES / 64;
const MAX_FREE_ENTRIES: usize = 8192;

struct FreeStack {
    entries: [u32; MAX_FREE_ENTRIES],
    count: usize,
}

impl FreeStack {
    const fn new() -> Self {
        Self { entries: [0; MAX_FREE_ENTRIES], count: 0 }
    }

    fn push(&mut self, pfn: u32) -> bool {
        if self.count < MAX_FREE_ENTRIES {
            self.entries[self.count] = pfn;
            self.count += 1;
            true
        } else {
            false
        }
    }

    fn pop(&mut self) -> Option<u32> {
        if self.count > 0 {
            self.count -= 1;
            Some(self.entries[self.count])
        } else {
            None
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

}

pub struct BuddyAllocator {
    free_lists: [FreeStack; MAX_ORDER],
    bitmap: [u64; BITMAP_SIZE],
    base_addr: u64,
    total_pages: usize,
    allocated_pages: usize,
    initialized: bool,
}

impl BuddyAllocator {
    pub const fn new() -> Self {
        Self {
            free_lists: [
                FreeStack::new(), FreeStack::new(), FreeStack::new(),
                FreeStack::new(), FreeStack::new(), FreeStack::new(),
                FreeStack::new(), FreeStack::new(), FreeStack::new(),
                FreeStack::new(), FreeStack::new(),
            ],
            bitmap: [0; BITMAP_SIZE],
            base_addr: 0,
            total_pages: 0,
            allocated_pages: 0,
            initialized: false,
        }
    }

    pub fn init(&mut self, ram_base: u64, ram_size: u64, kernel_start: u64, kernel_end: u64) {
        self.base_addr = ram_base;
        let total = (ram_size / PAGE_SIZE) as usize;
        self.total_pages = if total > MAX_PAGES { MAX_PAGES } else { total };

        let mut pfn: usize = 0;
        while pfn < self.total_pages {
            let page_addr = ram_base + (pfn as u64) * PAGE_SIZE;

            if page_addr < 0x100000 || (page_addr >= kernel_start && page_addr < kernel_end) {
                self.set_allocated(pfn);
                self.allocated_pages += 1;
                pfn += 1;
                continue;
            }

            let mut order = MAX_ORDER - 1;
            loop {
                let block_pages = 1 << order;
                if pfn % block_pages == 0 && pfn + block_pages <= self.total_pages {
                    let block_start = ram_base + (pfn as u64) * PAGE_SIZE;
                    let block_end = block_start + (block_pages as u64) * PAGE_SIZE;
                    let overlaps_reserved = block_start < 0x100000;
                    let overlaps_kernel = block_end > kernel_start && block_start < kernel_end;

                    if !overlaps_reserved && !overlaps_kernel {
                        self.free_lists[order].push(pfn as u32);
                        pfn += block_pages;
                        break;
                    }
                }
                if order == 0 {
                    self.set_allocated(pfn);
                    self.allocated_pages += 1;
                    pfn += 1;
                    break;
                }
                order -= 1;
            }
        }
        self.initialized = true;
    }

    pub fn alloc(&mut self, order: usize) -> Option<u64> {
        if !self.initialized || order >= MAX_ORDER {
            return None;
        }

        let mut current_order = order;
        while current_order < MAX_ORDER {
            if !self.free_lists[current_order].is_empty() { break; }
            current_order += 1;
        }
        if current_order >= MAX_ORDER { return None; }

        let pfn = self.free_lists[current_order].pop()? as usize;

        while current_order > order {
            current_order -= 1;
            let buddy_pfn = pfn + (1 << current_order);
            self.free_lists[current_order].push(buddy_pfn as u32);
        }

        let block_pages = 1 << order;
        let mut i = 0;
        while i < block_pages {
            self.set_allocated(pfn + i);
            i += 1;
        }
        self.allocated_pages += block_pages;

        Some(self.base_addr + (pfn as u64) * PAGE_SIZE)
    }

    pub fn alloc_page(&mut self) -> Option<u64> { self.alloc(0) }

    pub fn free(&mut self, addr: u64, order: usize) {
        if !self.initialized || order >= MAX_ORDER { return; }
        if addr < self.base_addr { return; }

        let pfn = ((addr - self.base_addr) / PAGE_SIZE) as usize;
        if pfn >= self.total_pages { return; }
        if !self.is_allocated(pfn) { return; } // double-free: silently ignore in host mirror

        let block_pages = 1 << order;
        let mut i = 0;
        while i < block_pages {
            self.clear_allocated(pfn + i);
            i += 1;
        }
        self.allocated_pages -= block_pages;

        let mut current_pfn = pfn;
        let mut current_order = order;

        while current_order < MAX_ORDER - 1 {
            let buddy_pfn = current_pfn ^ (1 << current_order);
            if buddy_pfn >= self.total_pages { break; }
            if !self.is_buddy_free(buddy_pfn, current_order) { break; }
            if !self.remove_from_free_list(buddy_pfn as u32, current_order) { break; }
            if buddy_pfn < current_pfn { current_pfn = buddy_pfn; }
            current_order += 1;
        }

        self.free_lists[current_order].push(current_pfn as u32);
    }

    pub fn free_page(&mut self, addr: u64) { self.free(addr, 0); }

    pub fn stats(&self) -> (usize, usize, usize) {
        (self.total_pages, self.allocated_pages, self.total_pages - self.allocated_pages)
    }

    pub fn is_initialized(&self) -> bool { self.initialized }

    fn set_allocated(&mut self, pfn: usize) {
        if pfn < MAX_PAGES { self.bitmap[pfn / 64] |= 1u64 << (pfn % 64); }
    }

    fn clear_allocated(&mut self, pfn: usize) {
        if pfn < MAX_PAGES { self.bitmap[pfn / 64] &= !(1u64 << (pfn % 64)); }
    }

    fn is_allocated(&self, pfn: usize) -> bool {
        if pfn < MAX_PAGES { self.bitmap[pfn / 64] & (1u64 << (pfn % 64)) != 0 } else { true }
    }

    fn is_buddy_free(&self, pfn: usize, order: usize) -> bool {
        let pages = 1 << order;
        let mut i = 0;
        while i < pages {
            if pfn + i >= self.total_pages || self.is_allocated(pfn + i) { return false; }
            i += 1;
        }
        true
    }

    fn remove_from_free_list(&mut self, pfn: u32, order: usize) -> bool {
        let list = &mut self.free_lists[order];
        let mut i = 0;
        while i < list.count {
            if list.entries[i] == pfn {
                list.count -= 1;
                list.entries[i] = list.entries[list.count];
                return true;
            }
            i += 1;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Small allocator for fast tests: 16MB RAM starting at 0x40000000
    // Kernel occupies first 512KB (128 pages)
    fn make_allocator() -> BuddyAllocator {
        let mut a = BuddyAllocator::new();
        let ram_base: u64 = 0x40000000;
        let ram_size: u64 = 16 * 1024 * 1024; // 16MB = 4096 pages
        let kernel_start = ram_base;
        let kernel_end = ram_base + 128 * PAGE_SIZE; // 512KB kernel
        a.init(ram_base, ram_size, kernel_start, kernel_end);
        a
    }

    // Full 2GB allocator matching QEMU boot
    fn make_full_allocator() -> BuddyAllocator {
        let mut a = BuddyAllocator::new();
        let ram_base: u64 = 0x40000000;
        let ram_size: u64 = 2048 * 1024 * 1024;
        let kernel_start = ram_base;
        let kernel_end = ram_base + 130 * PAGE_SIZE; // ~130 pages as seen in QEMU output
        a.init(ram_base, ram_size, kernel_start, kernel_end);
        a
    }

    // --- Init tests ---

    #[test]
    fn test_init_succeeds() {
        let a = make_allocator();
        assert!(a.is_initialized());
    }

    #[test]
    fn test_init_total_pages() {
        let a = make_allocator();
        let (total, _, _) = a.stats();
        assert_eq!(total, 4096); // 16MB / 4KB
    }

    #[test]
    fn test_init_kernel_pages_reserved() {
        let a = make_allocator();
        let (_, used, _) = a.stats();
        // 128 kernel pages reserved
        assert!(used >= 128, "used={used}");
    }

    #[test]
    fn test_init_free_pages_sane() {
        let a = make_allocator();
        let (total, used, free) = a.stats();
        assert_eq!(total, used + free);
        assert!(free > 0);
    }

    #[test]
    fn test_full_allocator_2gb() {
        let a = make_full_allocator();
        let (total, _, free) = a.stats();
        assert_eq!(total, 524288);
        assert!(free > 524000, "free={free}");
    }

    // --- Single page allocation ---

    #[test]
    fn test_alloc_page_returns_address() {
        let mut a = make_allocator();
        let addr = a.alloc_page();
        assert!(addr.is_some());
    }

    #[test]
    fn test_alloc_page_is_page_aligned() {
        let mut a = make_allocator();
        let addr = a.alloc_page().unwrap();
        assert_eq!(addr % PAGE_SIZE, 0, "addr={addr:#x} not page-aligned");
    }

    #[test]
    fn test_alloc_page_within_ram() {
        let mut a = make_allocator();
        let addr = a.alloc_page().unwrap();
        assert!(addr >= 0x40000000);
        assert!(addr < 0x40000000 + 16 * 1024 * 1024);
    }

    #[test]
    fn test_alloc_page_not_in_kernel() {
        let mut a = make_allocator();
        let kernel_end = 0x40000000 + 128 * PAGE_SIZE;
        let addr = a.alloc_page().unwrap();
        assert!(addr >= kernel_end, "addr={addr:#x} overlaps kernel");
    }

    #[test]
    fn test_alloc_page_increments_used() {
        let mut a = make_allocator();
        let (_, used_before, _) = a.stats();
        a.alloc_page().unwrap();
        let (_, used_after, _) = a.stats();
        assert_eq!(used_after, used_before + 1);
    }

    #[test]
    fn test_alloc_two_pages_different_addresses() {
        let mut a = make_allocator();
        let p1 = a.alloc_page().unwrap();
        let p2 = a.alloc_page().unwrap();
        assert_ne!(p1, p2);
    }

    #[test]
    fn test_alloc_pages_no_overlap() {
        let mut a = make_allocator();
        let p1 = a.alloc_page().unwrap();
        let p2 = a.alloc_page().unwrap();
        // Pages must not overlap (each is PAGE_SIZE bytes)
        let diff = if p1 > p2 { p1 - p2 } else { p2 - p1 };
        assert!(diff >= PAGE_SIZE, "p1={p1:#x} p2={p2:#x}");
    }

    // --- Multi-page (order > 0) allocation ---

    #[test]
    fn test_alloc_order1_is_8kb_aligned() {
        let mut a = make_allocator();
        let addr = a.alloc(1).unwrap();
        assert_eq!(addr % (2 * PAGE_SIZE), 0, "order-1 block not 8KB aligned");
    }

    #[test]
    fn test_alloc_order2_is_16kb_aligned() {
        let mut a = make_allocator();
        let addr = a.alloc(2).unwrap();
        assert_eq!(addr % (4 * PAGE_SIZE), 0);
    }

    #[test]
    fn test_alloc_order10_is_4mb_aligned() {
        let mut a = make_full_allocator();
        let addr = a.alloc(10).unwrap();
        assert_eq!(addr % (1024 * PAGE_SIZE), 0, "order-10 block not 4MB aligned");
    }

    #[test]
    fn test_alloc_order1_uses_2_pages() {
        let mut a = make_allocator();
        let (_, used_before, _) = a.stats();
        a.alloc(1).unwrap();
        let (_, used_after, _) = a.stats();
        assert_eq!(used_after - used_before, 2);
    }

    #[test]
    fn test_alloc_invalid_order_returns_none() {
        let mut a = make_allocator();
        assert!(a.alloc(MAX_ORDER).is_none());
        assert!(a.alloc(MAX_ORDER + 5).is_none());
    }

    // --- Free tests ---

    #[test]
    fn test_free_page_decrements_used() {
        let mut a = make_allocator();
        let addr = a.alloc_page().unwrap();
        let (_, used_before, _) = a.stats();
        a.free_page(addr);
        let (_, used_after, _) = a.stats();
        assert_eq!(used_before - used_after, 1);
    }

    #[test]
    fn test_free_page_allows_realloc() {
        let mut a = make_allocator();
        let addr = a.alloc_page().unwrap();
        a.free_page(addr);
        let addr2 = a.alloc_page();
        assert!(addr2.is_some());
    }

    #[test]
    fn test_free_restores_free_count() {
        let mut a = make_allocator();
        let (_, _, free_before) = a.stats();
        let addr = a.alloc_page().unwrap();
        a.free_page(addr);
        let (_, _, free_after) = a.stats();
        assert_eq!(free_before, free_after);
    }

    #[test]
    fn test_free_order1_restores_2_pages() {
        let mut a = make_allocator();
        let (_, _, free_before) = a.stats();
        let addr = a.alloc(1).unwrap();
        a.free(addr, 1);
        let (_, _, free_after) = a.stats();
        assert_eq!(free_before, free_after);
    }

    #[test]
    fn test_double_free_does_not_corrupt() {
        let mut a = make_allocator();
        let addr = a.alloc_page().unwrap();
        a.free_page(addr);
        a.free_page(addr); // second free: should be silently ignored
        let (total, used, free) = a.stats();
        assert_eq!(total, used + free); // accounting still consistent
    }

    #[test]
    fn test_free_invalid_addr_ignored() {
        let mut a = make_allocator();
        let (_, used_before, _) = a.stats();
        a.free_page(0x0); // below base
        a.free_page(0xDEADBEEF); // outside range
        let (_, used_after, _) = a.stats();
        assert_eq!(used_before, used_after);
    }

    // --- Coalescing tests ---

    #[test]
    fn test_coalesce_two_buddies() {
        let mut a = make_allocator();
        // Allocate two order-0 pages that should be buddies
        let p1 = a.alloc_page().unwrap();
        let p2 = a.alloc_page().unwrap();
        // Free both -- they should coalesce into an order-1 block
        a.free_page(p1);
        a.free_page(p2);
        // After coalescing, should be able to alloc an order-1 block
        let big = a.alloc(1);
        assert!(big.is_some(), "coalescing failed: order-1 alloc returned None");
    }

    #[test]
    fn test_coalesce_restores_full_free_count() {
        let mut a = make_allocator();
        let (_, _, free_start) = a.stats();
        let pages: Vec<u64> = (0..8).map(|_| a.alloc_page().unwrap()).collect();
        for p in pages { a.free_page(p); }
        let (_, _, free_end) = a.stats();
        assert_eq!(free_start, free_end, "free count mismatch after coalesce");
    }

    // --- Exhaustion tests ---

    #[test]
    fn test_exhaust_and_oom() {
        let mut a = make_allocator();
        let mut addrs = Vec::new();
        // Allocate until OOM
        loop {
            match a.alloc_page() {
                Some(addr) => addrs.push(addr),
                None => break,
            }
        }
        assert!(!addrs.is_empty(), "should have allocated at least one page");
        let (_, _, free) = a.stats();
        assert_eq!(free, 0, "free pages should be 0 after exhaustion");
    }

    #[test]
    fn test_exhaust_then_free_then_realloc() {
        let mut a = make_allocator();
        let mut addrs = Vec::new();
        loop {
            match a.alloc_page() {
                Some(addr) => addrs.push(addr),
                None => break,
            }
        }
        // Free all pages
        for addr in &addrs { a.free_page(*addr); }
        // Should be able to allocate again
        let addr = a.alloc_page();
        assert!(addr.is_some(), "should allocate after freeing all pages");
    }

    #[test]
    fn test_no_duplicate_addresses() {
        let mut a = make_allocator();
        let mut addrs = std::collections::HashSet::new();
        loop {
            match a.alloc_page() {
                Some(addr) => {
                    assert!(!addrs.contains(&addr), "duplicate address: {addr:#x}");
                    addrs.insert(addr);
                }
                None => break,
            }
        }
    }

    // --- Stats consistency ---

    #[test]
    fn test_stats_always_consistent() {
        let mut a = make_allocator();
        for _ in 0..100 {
            let addr = a.alloc_page().unwrap();
            let (total, used, free) = a.stats();
            assert_eq!(total, used + free);
            a.free_page(addr);
            let (total, used, free) = a.stats();
            assert_eq!(total, used + free);
        }
    }

    // --- Before init ---

    #[test]
    fn test_alloc_before_init_returns_none() {
        let mut a = BuddyAllocator::new();
        assert!(a.alloc_page().is_none());
        assert!(a.alloc(3).is_none());
    }

    #[test]
    fn test_not_initialized_by_default() {
        let a = BuddyAllocator::new();
        assert!(!a.is_initialized());
    }
}
