/* Veil kernel linker script — QEMU virt, RAM base 0x4000_0000, kernel at +1MiB. */
ENTRY(_start)

SECTIONS
{
    . = 0x40100000;
    __kernel_start = .;

    .text : {
        KEEP(*(.text.boot))
        *(.text*)
    }

    .rodata : ALIGN(8) {
        *(.rodata*)
    }

    .data : ALIGN(8) {
        *(.data*)
    }

    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    }

    /* Boot stack: 512 KiB above .bss, grows downward. */
    . = ALIGN(16);
    __stack_bottom = .;
    . += 0x80000;
    __stack_top = .;

    __kernel_end = .;

    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame*)
    }
}
