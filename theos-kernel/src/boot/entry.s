.section ".head.text", "ax"
.global _start
.type _start, @function
_start:
    b       kernel_entry
    .long   0
    .quad   0x0000000000000000
    .quad   0x0000000000200000
    .quad   0x000000000000000a
    .quad   0
    .quad   0
    .quad   0
    .ascii  "ARM\x64"
    .long   0

.section ".text._start", "ax"

.balign 2048
vector_table:
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler
    .balign 128
    b       exception_handler

exception_handler:
    wfe
    b       exception_handler

kernel_entry:
    /* CRITICAL: save DTB pointer from x0 BEFORE touching x0 */
    mov     x20, x0

    /* Install vector table -- uses x0 safely now */
    adrp    x0, vector_table
    add     x0, x0, :lo12:vector_table
    msr     vbar_el1, x0
    isb

    /* Set up stack */
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
    /* Pass DTB address to kernel_main */
    mov     x0, x20
    bl      kernel_main
hang:
    wfe
    b       hang
