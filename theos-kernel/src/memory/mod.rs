pub mod buddy;

use buddy::BuddyAllocator;

/// Global physical page allocator
static mut ALLOCATOR: BuddyAllocator = BuddyAllocator::new();

unsafe extern "C" {
    static THEOS_KERNEL_START: u8;
    static THEOS_KERNEL_END: u8;
}

/// Get a mutable pointer to the allocator without creating a reference
macro_rules! alloc_ptr {
    () => {
        unsafe { core::ptr::addr_of_mut!(ALLOCATOR).as_mut().unwrap_unchecked() }
    };
}

pub fn init(_dtb_paddr: u64) {
    let dtb_addr: u64 = 0x00000000;

    crate::println!("[memory] parsing DTB...");

    let info = unsafe { crate::hal::dtb::parse(dtb_addr) };

    match info {
        Ok(dtb) => {
            crate::println!("[memory] RAM: {} region(s), {} MB total",
                dtb.memory_region_count,
                dtb.total_ram_bytes() / (1024 * 1024)
            );

            let mut i = 0;
            while i < dtb.memory_region_count {
                let r = dtb.memory_regions[i];
                crate::println!("[memory]   region {}: {:#x}..{:#x} ({} MB)",
                    i, r.base, r.end(), r.size / (1024 * 1024)
                );
                i += 1;
            }

            crate::println!("[memory] UART: {:#x}  GIC: {:#x}/{:#x}",
                dtb.uart_base, dtb.gic_dist_base, dtb.gic_cpu_base
            );

            if dtb.memory_region_count > 0 {
                let region = dtb.memory_regions[0];
                let kernel_start = &raw const THEOS_KERNEL_START as u64;
                let kernel_end   = &raw const THEOS_KERNEL_END as u64;

                crate::println!("[memory] kernel: {:#x}..{:#x}", kernel_start, kernel_end);

                let a = alloc_ptr!();
                a.init(region.base, region.size, kernel_start, kernel_end);

                let (total, used, free) = a.stats();
                crate::println!("[memory] buddy: {} pages total, {} used, {} free",
                    total, used, free
                );

                // Self-test
                let page = a.alloc_page();
                match page {
                    Some(addr) => {
                        crate::println!("[memory] test alloc: {:#x} ok", addr);
                        a.free_page(addr);
                        crate::println!("[memory] test free: ok");
                    }
                    None => {
                        crate::println!("[memory] test alloc: FAILED");
                    }
                }
            }
        }
        Err(_) => {
            crate::println!("[memory] DTB parse failed");
        }
    }
}

pub fn alloc_page() -> Option<u64> {
    alloc_ptr!().alloc_page()
}

pub fn free_page(addr: u64) {
    alloc_ptr!().free_page(addr)
}

pub fn alloc(order: usize) -> Option<u64> {
    alloc_ptr!().alloc(order)
}

pub fn free(addr: u64, order: usize) {
    alloc_ptr!().free(addr, order)
}
