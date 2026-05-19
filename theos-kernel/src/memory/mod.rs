pub fn init(_dtb_paddr: u64) {
    // QEMU virt places DTB at 0x0
    let dtb_addr: u64 = 0x00000000;

    crate::println!("[memory] parsing DTB at {:#018x}", dtb_addr);

    let info = unsafe { crate::hal::dtb::parse(dtb_addr) };

    match info {
        Ok(dtb) => {
            crate::println!("[memory] DTB ok");
            crate::println!("[memory] RAM regions: {}", dtb.memory_region_count);
            let mut i = 0;
            while i < dtb.memory_region_count {
                let r = dtb.memory_regions[i];
                crate::println!(
                    "[memory]   [{}: base={:#018x} size={:#010x} ({} MB)]",
                    i, r.base, r.size, r.size / (1024 * 1024)
                );
                i += 1;
            }
            crate::println!("[memory] total RAM: {} MB", dtb.total_ram_bytes() / (1024 * 1024));
            crate::println!("[memory] UART:      {:#018x}", dtb.uart_base);
            crate::println!("[memory] GIC dist:  {:#018x}", dtb.gic_dist_base);
            crate::println!("[memory] GIC cpu:   {:#018x}", dtb.gic_cpu_base);
        }
        Err(crate::hal::dtb::DtbError::NullAddress) => {
            crate::println!("[memory] DTB null");
        }
        Err(crate::hal::dtb::DtbError::BadMagic(m)) => {
            crate::println!("[memory] DTB bad magic: {:#010x}", m);
        }
        Err(crate::hal::dtb::DtbError::BadVersion(v)) => {
            crate::println!("[memory] DTB bad version: {}", v);
        }
        Err(crate::hal::dtb::DtbError::OutOfBounds) => {
            crate::println!("[memory] DTB out of bounds");
        }
    }
}
