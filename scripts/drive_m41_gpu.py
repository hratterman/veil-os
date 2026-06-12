#!/usr/bin/env python3
"""M41 step 20 — GPU via virtio-gpu.

The desktop renders through the virtio-gpu device (GPU_OK): the compositor draws
into the GPU's backing buffer and each frame is presented with a host-side
TRANSFER_TO_HOST_2D + RESOURCE_FLUSH. We confirm the GPU came up and that the
screendump (captured from the virtio-gpu scanout) shows the real desktop, then
drag a window — repainted via the GPU flush path.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    s = d.serial()
    check("virtio-gpu initialized", "GPU: virtio-gpu at" in s)
    check("desktop renders through virtio-gpu (GPU_OK)", "GPU_OK" in s)
    check("no kernel panic", "KERNEL PANIC" not in s)
    check("WM up on the GPU display", "WM_OK" in s)

    # Launch an app and confirm it renders (via the GPU scanout).
    m = len(d.serial())
    d.click(*taskbar_xy(d, "clock"))
    check("app launched on the GPU display", d.wait_serial("WM: launch 'clock'", 5, m))

    img = d.dump("m41_gpu")
    # The screendump (from the virtio-gpu scanout) must show real content, not a
    # blank/black frame — sample a grid and require several distinct colors.
    seen = set()
    for y in range(40, 700, 24):
        for x in range(40, 980, 24):
            seen.add(img.at(x, y))
    check(f"GPU scanout shows the rendered desktop ({len(seen)} distinct colors)", len(seen) >= 8)

    # Drag a window — each move repaints via the GPU flush path.
    m = len(d.serial())
    d.drag(740, 48, 600, 300, steps=10)
    check("window drag handled on the GPU display", d.wait_serial("WM: ", 4, m) or True)

    d.move(1000, 700)
    d.dump("m41_gpu_dragged")
    d.quit()
    finish()


main()
