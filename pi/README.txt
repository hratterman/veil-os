Veil OS — Raspberry Pi 4 boot files (M17)

1. Format an SD card with a FAT32 first partition.
2. Copy the stock Pi 4 firmware onto it from
   https://github.com/raspberrypi/firmware/tree/master/boot :
     start4.elf  fixup4.dat  bcm2711-rpi-4-b.dtb
3. Copy kernel8.img and config.txt from this directory next to them.
4. HDMI monitor in the port nearest USB-C; optional serial console at
   115200 8N1 on GPIO 14 (TXD) / 15 (RXD) / GND.

The kernel prints BOOT_OK..M17_OK on serial and composites the Veil
desktop (shell / windows / paint) to the monitor.
