// Linux arm64 image header (Documentation/arch/arm64/booting.rst), placed
// at the very start of kernel8.img by linker-pi4.ld. The Pi firmware jumps
// to offset 0 (code0 branches over the header); loaders that parse the
// header (QEMU's -kernel, u-boot booti) honor text_offset and the magic.

.section .text.head, "ax"
.global _head

_head:
    b       _start              // code0: jump over the header
    .long   0                   // code1
    .quad   0x80000             // text_offset: load offset from RAM base
    .quad   __image_size        // image_size: kernel + bss + boot stack
    .quad   0b1010              // flags: LE, 4K pages, image anywhere in RAM
    .quad   0                   // res2
    .quad   0                   // res3
    .quad   0                   // res4
    .ascii  "ARM\x64"           // magic
    .long   0                   // res5
