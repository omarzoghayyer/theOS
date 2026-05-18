ENTRY(_start)

SECTIONS
{
    . = 0x40000000;

    .head.text : { KEEP(*(.head.text)) }

    .text : ALIGN(4096) {
        *(.text._start)
        *(.text*)
    }

    .rodata : ALIGN(4096) { *(.rodata*) }

    .data : ALIGN(4096) { *(.data*) }

    .bss : ALIGN(4096) {
        THEOS_BSS_START = .;
        *(.bss*)
        *(COMMON)
        THEOS_BSS_END = .;
    }

    . = ALIGN(16);
    THEOS_STACK_BOTTOM = .;
    . += 0x10000;
    THEOS_STACK_TOP = .;

    THEOS_KERNEL_END = .;

    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame*)
    }
}
