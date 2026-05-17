.section ".head.text", "ax"
.global _start
.type _start, @function

_start:
    b       kernel_entry
    .long   0
    .quad   0x80000
    .quad   0
    .quad   0x000a
    .quad   0
    .quad   0
    .quad   0
    .ascii  "ARM\x64"
    .long   0

.section ".text._start", "ax"
kernel_entry:
    /* Save DTB pointer */
    mov     x20, x0

    /* Set up stack using adrp (PC-relative, works for linker symbols) */
    adrp    x5, THEOS_STACK_TOP
    add     x5, x5, :lo12:THEOS_STACK_TOP
    mov     sp, x5

    /* Zero BSS */
    adrp    x6, THEOS_BSS_START
    add     x6, x6, :lo12:THEOS_BSS_START
    adrp    x7, THEOS_BSS_END
    add     x7, x7, :lo12:THEOS_BSS_END

bss_zero_loop:
    cmp     x6, x7
    b.ge    bss_zero_done
    str     xzr, [x6], #8
    b       bss_zero_loop

bss_zero_done:
    mov     x0, x20
    bl      kernel_main

hang:
    wfe
    b       hang
