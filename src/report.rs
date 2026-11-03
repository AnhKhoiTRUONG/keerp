use heapless::Vec;
use usbd_hid::descriptor::KeyboardUsage;

use crate::layer::ModifiersKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Report {
    pub modifiers: u8,
    pub key_codes: [u8; 6],
}

pub struct ReportBuilder {
    current: Report,
    out: Vec<Report, 16>,
}

impl ReportBuilder {
    pub const fn new() -> Self {
        Self {
            current: Report {
                modifiers: 0,
                key_codes: [0; 6],
            },
            out: Vec::new(),
        }
    }

    pub fn press(&mut self, code: KeyboardUsage) {
        let usage = code as u8;
        // if its already down
        if self.current.key_codes.contains(&usage) {
            return;
        }
        if (224..=231).contains(&usage) {
            self.current.modifiers |= 1 << (usage - 224);
            self.emit();
        } else if let Some(slot) = self.current.key_codes.iter_mut().find(|k| **k == 0) {
            *slot = usage;
            self.emit();
        }
        // TODO: 6 keys already down (rollover limit), ignore
    }

    pub fn release(&mut self, code: KeyboardUsage) {
        let usage = code as u8;
        if (224..=231).contains(&usage) {
            self.current.modifiers = 0;
            self.emit();
        } else if let Some(slot) = self.current.key_codes.iter_mut().find(|k| **k == usage) {
            *slot = 0;
            self.emit();
        }
    }

    pub fn press_mod(&mut self, m: ModifiersKey) {
        self.current.modifiers |= m as u8;
        self.emit();
    }

    pub fn release_mod(&mut self, m: ModifiersKey) {
        self.current.modifiers &= !(m as u8);
        self.emit();
    }

    /// The transport (or the test) drains the pending reports.
    pub fn take_reports(&mut self) -> Vec<Report, 16> {
        core::mem::take(&mut self.out)
    }

    fn emit(&mut self) {
        let _ = self.out.push(self.current); // TODO: full queue: drop (or handle)
    }
}
