//! The exe's input layers (bgm-input-01 Part B): device state becomes a
//! PS2-order PADDATA (FUN_140134d10 for XInput, FUN_140102ca0 for keyboard
//! and mouse), then a raw 64-bit word (FUN_14039bbe0), then logical bits
//! through the base table (0x1405c3420) and the current mode's table
//! (FUN_14039c720: an entry matches when all its raw bits are held), with
//! held / pressed / released / repeat fields (FUN_14039c4c0).

/// PADDATA button bits (PS2 DualShock order).
pub mod button {
    pub const SELECT: u16 = 0x1;
    pub const L3: u16 = 0x2;
    pub const R3: u16 = 0x4;
    pub const START: u16 = 0x8;
    pub const UP: u16 = 0x10;
    pub const RIGHT: u16 = 0x20;
    pub const DOWN: u16 = 0x40;
    pub const LEFT: u16 = 0x80;
    pub const L2: u16 = 0x100;
    pub const R2: u16 = 0x200;
    pub const L1: u16 = 0x400;
    pub const R1: u16 = 0x800;
    pub const TRIANGLE: u16 = 0x1000;
    pub const CIRCLE: u16 = 0x2000;
    pub const CROSS: u16 = 0x4000;
    pub const SQUARE: u16 = 0x8000;
}

/// Axa PADDATA: buttons and stick bytes (0x80 = centre; Y grows down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadData {
    pub buttons: u16,
    pub right: [u8; 2],
    pub left: [u8; 2],
}

impl Default for PadData {
    fn default() -> Self {
        Self {
            buttons: 0,
            right: [0x80; 2],
            left: [0x80; 2],
        }
    }
}

/// FUN_140132b50: an axis in -1..1 to a byte, with the Axa 0.2 dead zone.
pub fn axis_byte(f: f32) -> u8 {
    if f.abs() < 0.2 {
        0x80
    } else {
        (f.clamp(-1., 1.) * 127.5 + 127.5) as u8
    }
}

/// FUN_140133480 then FUN_140132b50: a stick vector (y down) stretched from
/// the circle to the square, clamped, to bytes.
pub fn stick_bytes(x: f32, y: f32) -> [u8; 2] {
    let len = (x * x + y * y).sqrt();
    let (mut x, mut y) = (x, y);
    let m = x.abs().max(y.abs());
    if len > 0.0157 && m > 0. {
        x = (x * len / m).clamp(-1., 1.);
        y = (y * len / m).clamp(-1., 1.);
    }
    [axis_byte(x), axis_byte(y)]
}

/// Default decide / back buttons (DAT_140782740 / 744, Western layout).
pub const DECIDE_BUTTON: u16 = button::CROSS;
pub const BACK_BUTTON: u16 = button::CIRCLE;
/// The game's stick dead zone in byte units (DAT_140bf3338).
pub const STICK_DEAD_ZONE: f32 = 60.;

/// FUN_14039bbe0: buttons, decide/back (bits 24/25), right-stick role
/// (bit 26 camera / 27 menu: L2 held flips it with the default config) and
/// stick direction bits.
pub fn raw(p: &PadData) -> u64 {
    let mut r = p.buttons as u64;
    if p.buttons & DECIDE_BUTTON != 0 {
        r |= 1 << 24;
    }
    if p.buttons & BACK_BUTTON != 0 {
        r |= 1 << 25;
    }
    // opt = ~(cfg >> 5) & 1 with the default cfg (bit 5 clear).
    let opt = 1;
    let v = ((!(p.buttons as u64 >> 8) & 1) ^ opt) + 1;
    r |= v * 0x400_0000;
    let dir = |b: u8| b as f32 - 127.5;
    let mut sticks = |bytes: [u8; 2], shift: u32| {
        let (x, y) = (dir(bytes[0]), dir(bytes[1]));
        for (on, bit) in [
            (x >= STICK_DEAD_ZONE, 0),
            (y >= STICK_DEAD_ZONE, 1),
            (x <= -STICK_DEAD_ZONE, 2),
            (y <= -STICK_DEAD_ZONE, 3),
        ] {
            if on {
                r |= 1 << (shift + bit);
            }
        }
    };
    sticks(p.right, 16);
    sticks(p.left, 20);
    r
}

/// The analog vector the game derives from stick bytes: per axis
/// sign x max(0, |b - 127.5| - 60) / 67.5, magnitude capped at 1.
pub fn analog(bytes: [u8; 2]) -> [f32; 3] {
    let axis = |b: u8| {
        let d = b as f32 - 127.5;
        d.signum() * (d.abs() - STICK_DEAD_ZONE).max(0.) / (127.5 - STICK_DEAD_ZONE)
    };
    let (x, y) = (axis(bytes[0]), axis(bytes[1]));
    [x, y, (x * x + y * y).sqrt().min(1.)]
}

/// Base table 0x1405c3420 (read from the exe image).
pub const BASE: &[(u64, u64)] = &[
    (0xf09, 0x1),
    (0x8, 0x2),
    (0x100_0000, 0x4),
    (0x200_0000, 0x8),
    (0x10, 0x10),
    (0x80_0000, 0x10),
    (0x40, 0x20),
    (0x20_0000, 0x20),
    (0x80, 0x40),
    (0x40_0000, 0x40),
    (0x20, 0x80),
    (0x10_0000, 0x80),
];

/// Mode 1, field and battle (0x1405c34f0).
pub const MODE_FIELD: &[(u64, u64)] = &[
    (0x200_0000, 0x100),
    (0x8000, 0x200),
    (0x1000, 0x400),
    (0x1, 0x40_0000),
    (0x10, 0x4000),
    (0x40, 0x8000),
    (0x80, 0x1_0000),
    (0x20, 0x1000_0000),
    (0x400, 0x80_0000),
    (0x200_0000, 0x100_0000),
    (0x1000, 0x200_0000),
    (0x8000, 0x400_0000),
    (0x100_0000, 0x800_0000),
    (0x800, 0x800),
    (0x400, 0x4_0000_0000),
    (0x100, 0x4_0000_0000),
    (0x800, 0x8_0000_0000),
    (0x200, 0x8_0000_0000),
    (0x200, 0x1_0000_0000),
    (0x4_0200, 0x10_0000_0000),
    (0x1_0200, 0x20_0000_0000),
    (0x408_0000, 0x4_0000),
    (0x402_0000, 0x8_0000),
    (0x404_0000, 0x10_0000),
    (0x401_0000, 0x20_0000),
    (0x408_0000, 0x2000_0000),
    (0x402_0000, 0x4000_0000),
    (0x400_0004, 0x2_0000),
    (0x808_0000, 0x4000),
    (0x802_0000, 0x8000),
    (0x804_0000, 0x1_0000),
    (0x801_0000, 0x1000_0000),
    (0x800_0004, 0x2_0000_0000),
];

/// Mode 7, title (0x1405c3c00).
pub const MODE_TITLE: &[(u64, u64)] = &[
    (0x8, 0x100),
    (0x1000, 0x200),
    (0x8, 0x400),
    (0x8000, 0x800),
    (0x1000, 0x1000),
    (0x1, 0x2000),
    (0x800, 0x4000),
    (0x400, 0x8000),
    (0x200, 0x1_0000),
    (0x100, 0x2_0000),
    (0x1_0000, 0x4_0000),
    (0x4_0000, 0x8_0000),
    (0x1000, 0x1_0000_0000),
    (0x8000, 0x2_0000_0000),
];

/// FUN_14039c720.
pub fn logical(raw: u64, mode: &[(u64, u64)]) -> u64 {
    BASE.iter()
        .chain(mode)
        .filter(|(mask, _)| raw & mask == *mask)
        .fold(0, |acc, (_, bits)| acc | bits)
}

/// First repeat after 30 frames, then every 4 (DAT_140bf333c / 340).
pub const REPEAT_DELAY: f32 = 30.;
pub const REPEAT_INTERVAL: f32 = 4.;

/// A pad record: held, new presses, releases and auto-repeat.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    pub held: u64,
    pub pressed: u64,
    pub released: u64,
    pub repeat: u64,
    timer: f32,
    /// The raw bits of the last update, so a mode change re-reads what is
    /// already held in the new mode instead of reporting it as pressed.
    raw: u64,
}

impl Pad {
    /// One frame from raw PADDATA bits in pad `mode`: presses are edges of
    /// the raw buttons, so switching modes (field START opening the camp,
    /// menu START closing it) does not turn a held button into a new press.
    pub fn update_raw(&mut self, raw: u64, mode: &[(u64, u64)], step: f32) {
        self.held = logical(self.raw, mode);
        self.raw = raw;
        self.update(logical(raw, mode), step);
    }

    pub fn update(&mut self, bits: u64, step: f32) {
        let previous = self.held;
        self.held = bits;
        self.pressed = bits & !previous;
        self.released = previous & !bits;
        if self.pressed != 0 || bits == 0 {
            self.timer = 0.;
            self.repeat = self.pressed;
            return;
        }
        self.timer += step;
        self.repeat = 0;
        if self.timer >= REPEAT_DELAY {
            self.repeat = bits;
            self.timer -= REPEAT_INTERVAL;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_button_is_not_a_new_press_after_a_mode_change() {
        let start = PadData {
            buttons: button::START,
            ..PadData::default()
        };
        let mut pad = Pad::default();
        pad.update_raw(raw(&start), MODE_FIELD, 1.);
        assert_ne!(pad.pressed, 0);
        // Still held on the next frame, now read in the menu's mode 7.
        pad.update_raw(raw(&start), MODE_TITLE, 1.);
        assert_eq!(pad.pressed, 0);
        assert_ne!(pad.held & 0x100, 0);
    }

    #[test]
    fn cross_decides_and_start_skips_on_the_title() {
        let p = PadData {
            buttons: button::CROSS,
            ..Default::default()
        };
        assert_eq!(logical(raw(&p), MODE_TITLE) & 0x4, 0x4);
        let p = PadData {
            buttons: button::START,
            ..Default::default()
        };
        assert_eq!(logical(raw(&p), MODE_TITLE) & 0x406, 0x402);
    }

    #[test]
    fn sticks_need_the_game_dead_zone() {
        // 0.45 passes Axa's 0.2 but stays inside the 60-unit zone.
        let p = PadData {
            left: stick_bytes(0., -0.45),
            ..Default::default()
        };
        assert_eq!(logical(raw(&p), MODE_TITLE) & 0x10, 0);
        let p = PadData {
            left: stick_bytes(0., -1.),
            ..Default::default()
        };
        assert_eq!(logical(raw(&p), MODE_TITLE) & 0x10, 0x10);
        assert_eq!(axis_byte(0.1), 0x80);
        assert_eq!(analog([255, 0x80]), [1., 0., 1.]);
    }

    #[test]
    fn right_stick_role_follows_l2() {
        let right = PadData {
            right: [255, 0x80],
            ..Default::default()
        };
        // Camera yaw right (bit 21) normally; menu toggle (bit 16) with L2.
        assert_eq!(logical(raw(&right), MODE_FIELD) & 0x20_0000, 0x20_0000);
        let menu = PadData {
            buttons: button::L2,
            ..right
        };
        assert_eq!(logical(raw(&menu), MODE_FIELD) & 0x1000_0000, 0x1000_0000);
    }

    #[test]
    fn lock_and_target_switch_use_distinct_field_input_edges() {
        let lock = PadData {
            buttons: button::R1,
            ..Default::default()
        };
        assert_ne!(logical(raw(&lock), MODE_FIELD) & 0x800, 0);
        let right = PadData {
            buttons: button::R2,
            right: [255, 128],
            ..Default::default()
        };
        let left = PadData {
            right: [0, 128],
            ..right
        };
        assert_eq!(logical(raw(&right), MODE_FIELD) & (3 << 36), 1 << 37);
        assert_eq!(logical(raw(&left), MODE_FIELD) & (3 << 36), 1 << 36);
        let only_shoulder = PadData {
            right: [128, 128],
            ..right
        };
        assert_eq!(logical(raw(&only_shoulder), MODE_FIELD) & (3 << 36), 0);
    }

    #[test]
    fn camera_chords_map_to_the_bits_the_lock_camera_reads() {
        let logical_of = |pad: PadData| logical(raw(&pad), MODE_FIELD);
        // Right stick up / down are logical bits 18 / 19 (FUN_140176bc0 zoom
        // in / out), left / right 20 / 21 (turn), R3 is bit 17 (recenter) and
        // R2 held is bit 32 (the zoom lock's override).
        let stick = |x: u8, y: u8| PadData {
            right: [x, y],
            ..Default::default()
        };
        assert_eq!(logical_of(stick(128, 0)) & (0xf << 18), 1 << 18);
        assert_eq!(logical_of(stick(128, 255)) & (0xf << 18), 1 << 19);
        assert_eq!(logical_of(stick(0, 128)) & (0xf << 18), 1 << 20);
        assert_eq!(logical_of(stick(255, 128)) & (0xf << 18), 1 << 21);
        let r3 = PadData {
            buttons: button::R3,
            ..Default::default()
        };
        assert_ne!(logical_of(r3) & (1 << 17), 0);
        let r2 = PadData {
            buttons: button::R2,
            ..Default::default()
        };
        assert_ne!(logical_of(r2) & (1 << 32), 0);
        assert_eq!(logical_of(PadData::default()) & (1 << 32), 0);
    }

    #[test]
    fn repeat_after_30_then_every_4() {
        let mut pad = Pad::default();
        let mut fired = vec![];
        for f in 0..40 {
            pad.update(0x10, 1.);
            if pad.repeat != 0 {
                fired.push(f);
            }
        }
        assert_eq!(fired, vec![0, 30, 34, 38]);
    }
}
