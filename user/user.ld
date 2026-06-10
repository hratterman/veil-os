/* Veil user binaries: flat images linked at USER_BASE. The kernel objcopies
 * these to raw .bin and maps them 1:1 at this VA. .bss must stay last: the
 * kernel maps zeroed pages beyond the binary image to cover it. */
ENTRY(_start)

SECTIONS
{
    . = 0x2000000000;
    .text : {
        KEEP(*(.text.start))
        *(.text*)
    }
    .rodata : ALIGN(8) { *(.rodata*) }
    .data : ALIGN(8) { *(.data*) }
    .bss (NOLOAD) : ALIGN(8) { *(.bss*) *(COMMON) }
    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
