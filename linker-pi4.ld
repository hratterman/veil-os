/* Veil kernel linker script — Raspberry Pi 4 (BCM2711).
   The Pi firmware loads kernel8.img at 0x8_0000 (arm_64bit=1); RAM base 0. */
ENTRY(_start)

SECTIONS
{
    . = 0x80000;
    __kernel_start = .;

    .text : {
        KEEP(*(.text.head))    /* Linux arm64 image header (pi4_head.s) */
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
    __image_size = __kernel_end - __kernel_start;

    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame*)
    }
}
