use embassy_time::{Duration, Instant};
use heapless::Vec;
use usbd_hid::descriptor::KeyboardUsage;

#[derive(Clone, Copy)]
pub struct KeyEvent {
    pub row: usize,
    pub col: usize,
    pub is_pressed: bool,
    pub time_ms: u64,
}

#[derive(Clone, Copy)]
pub enum KeyTouch {
    Key(KeyboardUsage),
    Chord(ModifiersKey, KeyboardUsage),
    LayerOn(LayerKey),
    HoldTap(LayerKey, KeyboardUsage),
    ModTap(ModifiersKey, KeyboardUsage),
    None,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LayerKey {
    Base = 0,
    Nav = 1,
    Num = 2,
    Sym = 3,
}

#[derive(Clone, Copy)]
pub enum ModifiersKey {
    LCTRL = 0x01,
    LSHIFT = 0x02,
    LALT = 0x04,
    LGUI = 0x08,
    RCTRL = 0x10,
    RSHIFT = 0x20,
    RALT = 0x40,
    RGUI = 0x80,
}

#[derive(Clone, Copy, PartialEq)]
pub enum HoldState {
    Idle,
    Waiting(Instant),
    Hold,
}
