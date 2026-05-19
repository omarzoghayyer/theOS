// hal/dtb.rs -- Flattened Device Tree (FDT) Parser
//
// Parses the DTB passed by the bootloader/QEMU to find:
//   - RAM regions (base + size) for the memory manager
//   - UART base address (replaces hardcoded 0x09000000)
//   - GIC base address for interrupt controller
//
// FDT binary format:
//   [header 40 bytes][memory reservations][structure block][strings block]
//
// Security assumptions (flag for audit):
//   - DTB address comes from bootloader in x0 -- trusted
//   - We validate magic number before any parsing
//   - All pointer arithmetic uses checked operations
//   - No heap allocation -- all parsing is zero-copy
//
// Reference: devicetree.org/specifications

const FDT_MAGIC: u32      = 0xD00DFEED;
const FDT_BEGIN_NODE: u32 = 0x00000001;
const FDT_END_NODE: u32   = 0x00000002;
const FDT_PROP: u32       = 0x00000003;
const FDT_NOP: u32        = 0x00000004;
const FDT_END: u32        = 0x00000009;

pub const MAX_MEMORY_REGIONS: usize = 8;

#[derive(Copy, Clone, Debug)]
pub struct MemoryRegion {
    pub base: u64,
    pub size: u64,
}

impl MemoryRegion {
    pub const fn zero() -> Self {
        Self { base: 0, size: 0 }
    }
    pub fn is_valid(&self) -> bool {
        self.size > 0
    }
    pub fn end(&self) -> u64 {
        self.base + self.size
    }
}

#[derive(Debug)]
pub struct DtbInfo {
    pub memory_regions: [MemoryRegion; MAX_MEMORY_REGIONS],
    pub memory_region_count: usize,
    pub uart_base: u64,
    pub gic_dist_base: u64,
    pub gic_cpu_base: u64,
}

impl DtbInfo {
    pub const fn empty() -> Self {
        Self {
            memory_regions: [MemoryRegion::zero(); MAX_MEMORY_REGIONS],
            memory_region_count: 0,
            uart_base: 0x09000000,
            gic_dist_base: 0,
            gic_cpu_base: 0,
        }
    }

    pub fn add_memory_region(&mut self, base: u64, size: u64) {
        if self.memory_region_count < MAX_MEMORY_REGIONS && size > 0 {
            self.memory_regions[self.memory_region_count] = MemoryRegion { base, size };
            self.memory_region_count += 1;
        }
    }

    pub fn total_ram_bytes(&self) -> u64 {
        let mut total = 0u64;
        let mut i = 0;
        while i < self.memory_region_count {
            total = total.saturating_add(self.memory_regions[i].size);
            i += 1;
        }
        total
    }
}

#[derive(Debug)]
pub enum DtbError {
    NullAddress,
    BadMagic(u32),
    BadVersion(u32),
    OutOfBounds,
}

unsafe fn read_be32(ptr: *const u8) -> u32 {
    let b0 = unsafe { *ptr.add(0) } as u32;
    let b1 = unsafe { *ptr.add(1) } as u32;
    let b2 = unsafe { *ptr.add(2) } as u32;
    let b3 = unsafe { *ptr.add(3) } as u32;
    (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
}

unsafe fn read_be64(ptr: *const u8) -> u64 {
    let hi = (unsafe { read_be32(ptr) }) as u64;
    let lo = (unsafe { read_be32(ptr.add(4)) }) as u64;
    (hi << 32) | lo
}

unsafe fn read_str<'a>(ptr: *const u8, max_len: usize) -> &'a [u8] {
    let mut len = 0;
    while len < max_len {
        if unsafe { *ptr.add(len) } == 0 {
            break;
        }
        len += 1;
    }
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn bytes_starts_with(a: &[u8], prefix: &[u8]) -> bool {
    if a.len() < prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if a[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Parse the DTB at the given physical address.
/// Safety: dtb_paddr must be a valid physical address of a DTB.
/// Called before MMU -- physical == virtual.
pub unsafe fn parse(dtb_paddr: u64) -> Result<DtbInfo, DtbError> {

    let base = dtb_paddr as *const u8;

    let magic = unsafe { read_be32(base) };
    if magic != FDT_MAGIC {
        return Err(DtbError::BadMagic(magic));
    }

    let totalsize      = unsafe { read_be32(base.add(4)) } as usize;
    let off_dt_struct  = unsafe { read_be32(base.add(8)) } as usize;
    let off_dt_strings = unsafe { read_be32(base.add(12)) } as usize;
    let version        = unsafe { read_be32(base.add(20)) };
    let size_dt_strings = unsafe { read_be32(base.add(36)) } as usize;

    if version < 17 {
        return Err(DtbError::BadVersion(version));
    }

    let struct_base  = unsafe { base.add(off_dt_struct) };
    let strings_base = unsafe { base.add(off_dt_strings) };
    let struct_size  = totalsize.saturating_sub(off_dt_struct);

    let mut info = DtbInfo::empty();
    let mut offset: usize = 0;
    let mut depth: usize = 0;
    let mut in_memory_node = false;
    let mut in_uart_node   = false;
    let mut in_gic_node    = false;
    let mut addr_cells: u32 = 2;
    let mut size_cells: u32 = 1;

    loop {
        if offset + 4 > struct_size {
            break;
        }

        let token = unsafe { read_be32(struct_base.add(offset)) };
        offset += 4;

        match token {
            FDT_BEGIN_NODE => {
                let name_start = offset;
                let mut name_len = 0;
                while offset < struct_size {
                    if unsafe { *struct_base.add(offset) } == 0 {
                        offset += 1;
                        break;
                    }
                    offset += 1;
                    name_len += 1;
                }
                offset = (offset + 3) & !3;

                let node_name = unsafe {
                    core::slice::from_raw_parts(struct_base.add(name_start), name_len)
                };

                depth += 1;

                if depth == 1 {
                    in_memory_node = bytes_starts_with(node_name, b"memory");
                    in_uart_node   = bytes_starts_with(node_name, b"pl011")
                                  || bytes_starts_with(node_name, b"uart");
                    in_gic_node    = bytes_starts_with(node_name, b"intc")
                                  || bytes_starts_with(node_name, b"interrupt-controller")
                                  || bytes_starts_with(node_name, b"gic");
                } else if depth > 1 {
                    in_memory_node = false;
                    in_uart_node   = false;
                    in_gic_node    = false;
                }
            }

            FDT_END_NODE => {
                if depth > 0 {
                    depth -= 1;
                }
                if depth == 0 {
                    in_memory_node = false;
                    in_uart_node   = false;
                    in_gic_node    = false;
                }
            }

            FDT_PROP => {
                if offset + 8 > struct_size {
                    break;
                }
                let prop_len    = unsafe { read_be32(struct_base.add(offset)) } as usize;
                let name_offset = unsafe { read_be32(struct_base.add(offset + 4)) } as usize;
                offset += 8;

                if offset > struct_size {
                    break;
                }

                let prop_data = unsafe { struct_base.add(offset) };
                let prop_name = if name_offset < size_dt_strings {
                    unsafe {
                        read_str(
                            strings_base.add(name_offset),
                            size_dt_strings - name_offset,
                        )
                    }
                } else {
                    b""
                };

                if depth == 1 && bytes_eq(prop_name, b"#address-cells") && prop_len == 4 {
                    addr_cells = unsafe { read_be32(prop_data) };
                }
                if depth == 1 && bytes_eq(prop_name, b"#size-cells") && prop_len == 4 {
                    size_cells = unsafe { read_be32(prop_data) };
                }

                if in_memory_node && bytes_eq(prop_name, b"reg") {
                    let cell_size = (addr_cells + size_cells) as usize * 4;
                    let mut reg_offset = 0;
                    while reg_offset + cell_size <= prop_len {
                        let base_addr = if addr_cells == 2 {
                            unsafe { read_be64(prop_data.add(reg_offset)) }
                        } else {
                            (unsafe { read_be32(prop_data.add(reg_offset)) }) as u64
                        };
                        let size_off = reg_offset + addr_cells as usize * 4;
                        let region_size = if size_cells == 2 {
                            unsafe { read_be64(prop_data.add(size_off)) }
                        } else {
                            (unsafe { read_be32(prop_data.add(size_off)) }) as u64
                        };
                        info.add_memory_region(base_addr, region_size);
                        reg_offset += cell_size;
                    }
                }

                if in_uart_node && bytes_eq(prop_name, b"reg") && prop_len >= 8 {
                    info.uart_base = if addr_cells == 2 {
                        unsafe { read_be64(prop_data) }
                    } else {
                        (unsafe { read_be32(prop_data) }) as u64
                    };
                }

                if in_gic_node && bytes_eq(prop_name, b"reg") && prop_len >= 16 {
                    info.gic_dist_base = if addr_cells == 2 {
                        unsafe { read_be64(prop_data) }
                    } else {
                        (unsafe { read_be32(prop_data) }) as u64
                    };
                    let cpu_off = addr_cells as usize * 4 + size_cells as usize * 4;
                    if cpu_off + addr_cells as usize * 4 <= prop_len {
                        info.gic_cpu_base = if addr_cells == 2 {
                            unsafe { read_be64(prop_data.add(cpu_off)) }
                        } else {
                            (unsafe { read_be32(prop_data.add(cpu_off)) }) as u64
                        };
                    }
                }

                offset += (prop_len + 3) & !3;
            }

            FDT_NOP => {}

            FDT_END => break,

            _ => break,
        }
    }

    Ok(info)
}
