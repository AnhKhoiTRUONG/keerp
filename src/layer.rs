use usbd_hid::descriptor::KeyboardUsage;

pub struct Layout<const L: usize> {
    pub keymap: [[[KeyboardUsage; 10]; 4]; L],
    pub idx: usize,
}

impl<const L: usize> Layout<L> {
    pub fn new(keymap: [[[KeyboardUsage; 10]; 4]; L]) -> Self {
        Self { keymap, idx: 0 }
    }

    pub fn change_layout(&mut self) {
        self.idx = (self.idx + 1) % L;
    }

    pub fn ret_key(&self, i: usize, j: usize) -> KeyboardUsage {
        self.keymap[self.idx][i][j]
    }
}
