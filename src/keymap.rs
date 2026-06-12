//! Linux evdev keycode -> ASCII, US layout, enough for a shell and Paint.

/// `None` for keys with no character meaning (modifiers, F-keys, ...).
/// Backspace maps to 0x08, tab to '\t', enter to '\n'.
pub fn translate(code: u16, shift: bool) -> Option<char> {
    const TABLE: &[(u16, char, char)] = &[
        (2, '1', '!'), (3, '2', '@'), (4, '3', '#'), (5, '4', '$'),
        (6, '5', '%'), (7, '6', '^'), (8, '7', '&'), (9, '8', '*'),
        (10, '9', '('), (11, '0', ')'), (12, '-', '_'), (13, '=', '+'),
        (14, '\u{8}', '\u{8}'), (15, '\t', '\t'),
        (16, 'q', 'Q'), (17, 'w', 'W'), (18, 'e', 'E'), (19, 'r', 'R'),
        (20, 't', 'T'), (21, 'y', 'Y'), (22, 'u', 'U'), (23, 'i', 'I'),
        (24, 'o', 'O'), (25, 'p', 'P'), (26, '[', '{'), (27, ']', '}'),
        (28, '\n', '\n'),
        (30, 'a', 'A'), (31, 's', 'S'), (32, 'd', 'D'), (33, 'f', 'F'),
        (34, 'g', 'G'), (35, 'h', 'H'), (36, 'j', 'J'), (37, 'k', 'K'),
        (38, 'l', 'L'), (39, ';', ':'), (40, '\'', '"'), (41, '`', '~'),
        (43, '\\', '|'),
        (44, 'z', 'Z'), (45, 'x', 'X'), (46, 'c', 'C'), (47, 'v', 'V'),
        (48, 'b', 'B'), (49, 'n', 'N'), (50, 'm', 'M'), (51, ',', '<'),
        (52, '.', '>'), (53, '/', '?'), (57, ' ', ' '),
    ];
    TABLE
        .iter()
        .find(|&&(c, _, _)| c == code)
        .map(|&(_, lower, upper)| if shift { upper } else { lower })
}

pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_TAB: u16 = 15;
pub const KEY_A: u16 = 30;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
pub const KEY_0: u16 = 11;
pub const KEY_MINUS: u16 = 12;
pub const KEY_EQUAL: u16 = 13;
pub const KEY_SYSRQ: u16 = 99; // Print Screen
pub const KEY_W: u16 = 17;
pub const KEY_T: u16 = 20;
pub const KEY_F: u16 = 33;
pub const KEY_N: u16 = 49;
pub const KEY_F3: u16 = 61;
pub const KEY_F4: u16 = 62;
pub const KEY_F5: u16 = 63;
pub const KEY_F11: u16 = 87;

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
pub const EV_ABS: u16 = 3;
pub const ABS_X: u16 = 0;
pub const ABS_Y: u16 = 1;
pub const REL_WHEEL: u16 = 8; // mouse/tablet scroll wheel (signed notches)
