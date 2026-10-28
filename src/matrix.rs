use embassy_rp::gpio::{AnyPin, Input, Level, Output, Pin, Pull};
use embassy_rp::Peri;
use embassy_time::Timer;
use usbd_hid::descriptor::KeyboardUsage;

pub struct Matrix<'d, const CS: usize, const RS: usize> {
    cols: [Input<'d>; CS],
    rows: [Output<'d>; RS],
}

pub fn pin_rows_to_output_rows<const RS: usize>(
    rows: [Peri<'static, AnyPin>; RS],
) -> [Output<'static>; RS] {
    rows.map(|p| Output::new(p, Level::High))
}

pub fn pin_cols_to_input_cols<const CS: usize>(
    cols: [Peri<'static, AnyPin>; CS],
) -> [Input<'static>; CS] {
    cols.map(|p| Input::new(p, Pull::Up))
}

impl<'d, const CS: usize, const RS: usize> Matrix<'d, CS, RS> {
    pub fn new(cols: [Input<'d>; CS], rows: [Output<'d>; RS]) -> Self {
        Self { cols, rows }
    }

    pub async fn scan_a_test(&mut self) -> bool {
        for r in self.rows.iter_mut() {
            r.set_low();
            Timer::after_millis(30).await;
            for c in &self.cols {
                if c.is_low() {
                    return true;
                }
            }
            r.set_high();
        }
        false
    }

    pub async fn scan(&mut self) -> [[bool; CS]; RS] {
        let mut keys = [[false; CS]; RS];
        for (i, r) in self.rows.iter_mut().enumerate() {
            r.set_low();
            Timer::after_millis(10).await;
            for (j, c) in self.cols.iter().enumerate() {
                if c.is_low() {
                    keys[i][j] = true;
                }
            }
            r.set_high();
        }
        keys
    }
}
