// memory/buddy.rs -- Buddy Allocator for Physical Pages
//
// Classic buddy allocation scheme for 4KB physical pages.
//
// Design:
//   - MAX_ORDER = 10 -> max block = 2^10 * 4KB = 4MB
//   - Free list per order: free_lists[0] = 4KB, free_lists[1] = 8KB, ...
//   - Bitmap tracks whether each page is allocated
//   - Split larger blocks to satisfy smaller requests
//   - Coalesce adjacent buddies on free
//
// Memory layout:
//   The allocator metadata (bitmap + free lists) lives in a static array.
//   We track up to MAX_PAGES pages = 512K pages = 2GB max RAM.
//
// Security:
//   All allocations are page-aligned.
//   Double-free is detected and rejected.
//   No heap needed -- all metadata is static.

/// Page size: 4KB
pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;

/// Maximum order: 2^10 pages = 4MB blocks
pub const MAX_ORDER: usize = 11;

/// Maximum pages we track (2GB / 4KB = 524288)
const MAX_PAGES: usize = 524288;

/// Bitmap: 1 bit per page, packed into u64s
const BITMAP_SIZE: usize = MAX_PAGES / 64;

/// A node in the free list -- stored at the start of the free block itself
/// Since we have no heap, we use the free page's own memory for the linked list.
/// This works because free pages are unused memory.
///
/// But we can't dereference physical addresses as pointers to structs easily
/// without the MMU. So instead we use a simple array-based free list:
/// free_lists[order] is a stack of page frame numbers (PFNs).

/// Maximum entries per free list
const MAX_FREE_ENTRIES: usize = 8192;

/// Free list for one order -- a stack of page frame numbers
struct FreeStack {
    entries: [u32; MAX_FREE_ENTRIES],
    count: usize,
}

impl FreeStack {
    const fn new() -> Self {
        Self {
            entries: [0; MAX_FREE_ENTRIES],
            count: 0,
        }
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

/// The buddy allocator -- all state is static
pub struct BuddyAllocator {
    /// Free lists per order
    free_lists: [FreeStack; MAX_ORDER],
    /// Bitmap: bit set = page is allocated
    bitmap: [u64; BITMAP_SIZE],
    /// Base physical address of managed RAM
    base_addr: u64,
    /// Total pages managed
    total_pages: usize,
    /// Pages currently allocated
    allocated_pages: usize,
    /// Whether init has been called
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

    /// Initialize the allocator with a RAM region.
    /// Reserves pages occupied by the kernel (kernel_start..kernel_end).
    pub fn init(&mut self, ram_base: u64, ram_size: u64, kernel_start: u64, kernel_end: u64) {
        self.base_addr = ram_base;

        // Calculate total pages (capped at MAX_PAGES)
        let total = (ram_size / PAGE_SIZE) as usize;
        self.total_pages = if total > MAX_PAGES { MAX_PAGES } else { total };

        // Add all pages as free blocks at the highest order possible
        let mut pfn: usize = 0;
        while pfn < self.total_pages {
            let page_addr = ram_base + (pfn as u64) * PAGE_SIZE;

            // Skip kernel-occupied pages and first 1MB (DTB + reserved)
            if page_addr < 0x100000 || (page_addr >= kernel_start && page_addr < kernel_end) {
                self.set_allocated(pfn);
                self.allocated_pages += 1;
                pfn += 1;
                continue;
            }

            // Find the largest order block starting at this PFN
            let mut order = MAX_ORDER - 1;
            loop {
                let block_pages = 1 << order;
                // Block must be naturally aligned and fit in RAM
                if pfn % block_pages == 0 && pfn + block_pages <= self.total_pages {
                    // Check that no page in this block overlaps kernel or reserved
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
                    // Can't place even a single page -- skip
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

    /// Allocate 2^order contiguous pages.
    /// Returns physical address of the allocated block, or None.
    pub fn alloc(&mut self, order: usize) -> Option<u64> {
        if !self.initialized || order >= MAX_ORDER {
            return None;
        }

        // Find the smallest order with a free block >= requested
        let mut current_order = order;
        while current_order < MAX_ORDER {
            if !self.free_lists[current_order].is_empty() {
                break;
            }
            current_order += 1;
        }

        if current_order >= MAX_ORDER {
            return None; // Out of memory
        }

        // Pop a block from this order
        let pfn = self.free_lists[current_order].pop()? as usize;

        // Split down to requested order
        while current_order > order {
            current_order -= 1;
            let buddy_pfn = pfn + (1 << current_order);
            self.free_lists[current_order].push(buddy_pfn as u32);
        }

        // Mark pages as allocated
        let block_pages = 1 << order;
        let mut i = 0;
        while i < block_pages {
            self.set_allocated(pfn + i);
            i += 1;
        }
        self.allocated_pages += block_pages;

        let addr = self.base_addr + (pfn as u64) * PAGE_SIZE;
        Some(addr)
    }

    /// Allocate a single 4KB page.
    pub fn alloc_page(&mut self) -> Option<u64> {
        self.alloc(0)
    }

    /// Free a block of 2^order pages at the given physical address.
    pub fn free(&mut self, addr: u64, order: usize) {
        if !self.initialized || order >= MAX_ORDER {
            return;
        }
        if addr < self.base_addr {
            return;
        }

        let pfn = ((addr - self.base_addr) / PAGE_SIZE) as usize;
        if pfn >= self.total_pages {
            return;
        }

        // Check for double-free
        if !self.is_allocated(pfn) {
            crate::println!("[memory] DOUBLE FREE at {:#018x}", addr);
            return;
        }

        // Mark pages as free
        let block_pages = 1 << order;
        let mut i = 0;
        while i < block_pages {
            self.clear_allocated(pfn + i);
            i += 1;
        }
        self.allocated_pages -= block_pages;

        // Coalesce with buddy
        let mut current_pfn = pfn;
        let mut current_order = order;

        while current_order < MAX_ORDER - 1 {
            let buddy_pfn = current_pfn ^ (1 << current_order);

            // Check buddy is in range and entirely free
            if buddy_pfn >= self.total_pages {
                break;
            }

            let buddy_free = self.is_buddy_free(buddy_pfn, current_order);
            if !buddy_free {
                break;
            }

            // Remove buddy from its free list
            if !self.remove_from_free_list(buddy_pfn as u32, current_order) {
                break;
            }

            // Merge: take the lower PFN
            if buddy_pfn < current_pfn {
                current_pfn = buddy_pfn;
            }
            current_order += 1;
        }

        self.free_lists[current_order].push(current_pfn as u32);
    }

    /// Free a single 4KB page.
    pub fn free_page(&mut self, addr: u64) {
        self.free(addr, 0);
    }

    /// Returns (total_pages, allocated_pages, free_pages)
    pub fn stats(&self) -> (usize, usize, usize) {
        (self.total_pages, self.allocated_pages, self.total_pages - self.allocated_pages)
    }

    // --- Internal helpers ---

    fn set_allocated(&mut self, pfn: usize) {
        if pfn < MAX_PAGES {
            self.bitmap[pfn / 64] |= 1u64 << (pfn % 64);
        }
    }

    fn clear_allocated(&mut self, pfn: usize) {
        if pfn < MAX_PAGES {
            self.bitmap[pfn / 64] &= !(1u64 << (pfn % 64));
        }
    }

    fn is_allocated(&self, pfn: usize) -> bool {
        if pfn < MAX_PAGES {
            self.bitmap[pfn / 64] & (1u64 << (pfn % 64)) != 0
        } else {
            true
        }
    }

    fn is_buddy_free(&self, pfn: usize, order: usize) -> bool {
        let pages = 1 << order;
        let mut i = 0;
        while i < pages {
            if pfn + i >= self.total_pages || self.is_allocated(pfn + i) {
                return false;
            }
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
