import sys, time
from guilib import Driver, check, finish
# No-NIC desktop: icon_order defaults to
#   col0 (x=8):  edit clock browser paint shell      (slots 0..4)
#   col1 (x=76): viewer audio files gif lisp          (slots 5..9)
# Icon slot 0 (edit) center ~ (32,32); slot 1 (clock) center ~ (32,100).
def btn(d, down):
    d.send([{"type": "btn", "data": {"down": down, "button": "left"}}])
def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.move(32, 32)          # cursor over the 'edit' icon
    btn(d, True)            # press and hold
    time.sleep(0.45)        # hold >200ms -> promotes to a drag
    check("hold promoted to drag", d.wait_serial("WM: icon drag start 'edit'", 3, m))
    # Drag down to the 'clock' slot (slot 1), a few steps so it tracks.
    for yy in (48, 64, 80, 96, 100):
        d.move(32, yy); time.sleep(0.03)
    m2 = len(d.serial())
    btn(d, False)           # drop
    check("DRAG_OK emitted", d.wait_serial("DRAG_OK", 3, m2))
    check("edit dropped at slot 1", d.wait_serial("WM: icon 'edit' dropped at slot 1", 3, m2))
    # Order now persisted to ICONS.TXT as: clock edit browser ...
    d.quit(); finish()
main()
