use crate::{
    layer::{
        KeyEvent,
        KeyTouch::{self, Key},
        LayerKey, ModifiersKey,
    },
    report::{Report, ReportBuilder},
};
use heapless::Vec;
use usbd_hid::descriptor::KeyboardUsage;

// Here we implemement the balanced hold tap behavior
// The interrupting key press is not handled immediatly
// The decision is made when:
//          - TAPPING_TERM_MS is expired
//          - The interrupting key is released
//          - The hold tap key is released

const TAPPING_TERM_MS: u64 = 180;

pub struct Engine<const L: usize> {
    pub keymap: [[[KeyTouch; 10]; 4]; L],
    pub report: ReportBuilder,
    pub bound: [[Option<KeyTouch>; 10]; 4],
    pub layout: LayerKey,
    state: HoldState,
    queue: Vec<KeyEvent, 8>,
}

#[derive(Clone, Copy)]
enum HoldState {
    Idle,
    Pending {
        row: usize,
        col: usize,
        tap: KeyboardUsage,
        hold_mod: Option<ModifiersKey>,
        hold_layer: Option<LayerKey>,
        deadline: u64,
    },
}

impl HoldState {
    pub fn set_idle(&mut self) {
        *self = HoldState::Idle;
    }
    pub fn set_pending(
        &mut self,
        row: usize,
        col: usize,
        tap: KeyboardUsage,
        hold_mod: Option<ModifiersKey>,
        hold_layer: Option<LayerKey>,
        deadline: u64,
    ) {
        *self = HoldState::Pending {
            row,
            col,
            tap,
            hold_mod,
            hold_layer,
            deadline,
        }
    }
}

impl<const L: usize> Engine<L> {
    pub fn new(keymap: [[[KeyTouch; 10]; 4]; L]) -> Self {
        Self {
            keymap,
            report: ReportBuilder::new(),
            bound: [[None; 10]; 4],
            layout: LayerKey::Base,
            state: HoldState::Idle,
            queue: Vec::new(),
        }
    }

    // we check if we should process the waiting hold tap or process a normal key
    pub fn handle(&mut self, e: KeyEvent) {
        if self.offer(e) {
            return;
        }
        self.process(e);
    }

    pub fn process(&mut self, e: KeyEvent) {
        if e.is_pressed {
            self.on_press(e);
        } else {
            self.on_release(e);
        }
    }

    /// Returns true if the event was consumed.
    fn offer(&mut self, e: KeyEvent) -> bool {
        let HoldState::Pending {
            row, col, deadline, ..
        } = self.state
        else {
            return false;
        };

        // process e normally, now that the hold is active
        if e.time_ms >= deadline {
            self.resolve_hold();
            return false;
        }

        if !e.is_pressed {
            // the hold tap key was released -> TAP
            if (e.row, e.col) == (row, col) {
                self.resolve_tap();
                true
            } else {
                // Key released when there's a holding waiting for decision:
                //      - We check if it is pressed before so we decide the holding key as hold
                //      else do nothing
                if self
                    .queue
                    .iter()
                    .any(|q| (q.row, q.col) == (e.row, e.col) && q.is_pressed)
                {
                    if self.queue.push(e).is_err() {
                        self.resolve_hold();
                        return self.offer(e);
                    }
                    self.resolve_hold();
                    true
                } else {
                    false
                }
            }
        } else {
            if (e.row, e.col) == (row, col) {
                true
            }
            // if another key is pressed we push into the queue waiting for decision
            else {
                // if the key is full, we make a decision now
                if self.queue.push(e).is_err() {
                    self.resolve_hold();
                    return self.offer(e);
                }
                true
            }
        }
    }

    fn resolve_tap(&mut self) {
        let HoldState::Pending { tap, .. } = self.state else {
            return;
        };
        self.state.set_idle();
        self.send_tap(tap);
        self.replay();
    }

    fn resolve_hold(&mut self) {
        let HoldState::Pending {
            row,
            col,
            hold_mod,
            hold_layer,
            tap,
            ..
        } = self.state
        else {
            return;
        };
        self.state.set_idle();
        if let Some(m) = hold_mod {
            self.bound[row][col] = Some(KeyTouch::ModTap(m, tap));
            self.report.press_mod(m);
        } else if let Some(l) = hold_layer {
            self.bound[row][col] = Some(KeyTouch::HoldTap(l, tap));
            self.layout = l;
        }
        self.replay();
    }

    // This function is called when the decision is made (state is Idle)
    // so it will just do everything in the queue in order
    // Only called after resolve_hold and resolve_tap
    fn replay(&mut self) {
        let queued = core::mem::take(&mut self.queue);
        for ev in queued {
            self.handle(ev);
        }
    }

    /// Call when the firmware timer fires.
    pub fn on_timeout(&mut self, now: u64) {
        if let HoldState::Pending { deadline, .. } = self.state {
            if now >= deadline {
                self.resolve_hold();
            }
        }
    }

    /// The firmware uses this to arm its timer.
    pub fn next_deadline(&self) -> Option<u64> {
        match self.state {
            HoldState::Pending { deadline, .. } => Some(deadline),
            HoldState::Idle => None,
        }
    }

    fn send_tap(&mut self, k: KeyboardUsage) {
        self.report.press(k);
        self.report.release(k);
    }

    fn on_press(&mut self, e: KeyEvent) {
        if self.bound[e.row][e.col].is_some() {
            return; // duplicate press without a release: ignore
        }
        match self.lookup(e.row, e.col) {
            KeyTouch::Key(k) => {
                self.bound[e.row][e.col] = Some(KeyTouch::Key(k));
                self.report.press(k);
            }
            KeyTouch::ModTap(m, k) => {
                self.state
                    .set_pending(e.row, e.col, k, Some(m), None, e.time_ms + TAPPING_TERM_MS);
            }
            KeyTouch::HoldTap(l, k) => {
                self.state
                    .set_pending(e.row, e.col, k, None, Some(l), e.time_ms + TAPPING_TERM_MS);
            }
            _ => {}
        }
    }

    fn on_release(&mut self, e: KeyEvent) {
        // take() clears the slot, so each press is released exactly once
        match self.bound[e.row][e.col].take() {
            Some(KeyTouch::Key(k)) => self.report.release(k),
            Some(KeyTouch::ModTap(m, _)) => self.report.release_mod(m),
            Some(KeyTouch::HoldTap(_, _)) => self.change_layout(LayerKey::Base),

            _ => {} // stray release
        }
    }

    pub fn take_reports(&mut self) -> Vec<Report, 16> {
        self.report.take_reports()
    }

    pub fn change_layout(&mut self, layout: LayerKey) {
        self.layout = layout;
    }

    pub fn lookup(&self, i: usize, j: usize) -> KeyTouch {
        self.keymap[self.layout as usize][i][j]
    }

    pub fn current_layout(&self) -> LayerKey {
        self.layout
    }
}
