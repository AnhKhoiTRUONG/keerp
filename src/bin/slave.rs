//! This example shows how to use UART (Universal asynchronous receiver-transmitter) in the RP2040 chip.
//!
//! No specific hardware is specified in this example. Only output on pin 0 is tested.
//! The Raspberry Pi Debug Probe (https://www.raspberrypi.com/products/debug-probe/) could be used
//! with its UART port.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::DMA_CH0;
use embassy_rp::uart::{self, UartTx};
use embassy_time::Timer;
use keerp::matrix;
use panic_halt as _;

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<DMA_CH0>;
});

/*The data send through uart have the format
* [0x67, row, cols, is_pressed, 0x67]
* to make sure that we dont have decalage*/

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let config = uart::Config::default();
    let mut uart = UartTx::new(p.UART0, p.PIN_0, p.DMA_CH0, Irqs, config);

    let mut led = Output::new(p.PIN_25, Level::Low);

    let rows = [
        Output::new(p.PIN_26, Level::High),
        Output::new(p.PIN_27, Level::High),
        Output::new(p.PIN_28, Level::High),
        Output::new(p.PIN_29, Level::High),
    ];

    // 5 Columns initialized directly into the array as Inputs
    let cols = [
        Input::new(p.PIN_2, Pull::Up),
        Input::new(p.PIN_4, Pull::Up),
        Input::new(p.PIN_3, Pull::Up),
        Input::new(p.PIN_7, Pull::Up),
        Input::new(p.PIN_6, Pull::Up),
    ];

    let mut prev_keys = [[false; 5]; 4];
    let mut keys;

    let mut matrix_key = matrix::Matrix::new(cols, rows);

    loop {
        keys = matrix_key.scan().await;

        if prev_keys != keys {
            for i in 0..4 {
                for j in 0..5 {
                    if keys[i][j] != prev_keys[i][j] {
                        let state_byte = if keys[i][j] { 1 } else { 0 };
                        let message: [u8; 5] = [0x67, i as u8, j as u8, state_byte, 0x67];
                        let _ = uart.blocking_write(&message);
                        if led.is_set_high() {
                            led.set_low();
                        } else {
                            led.set_high();
                        }
                    }
                }
            }
            prev_keys = keys;
        }
        Timer::after_millis(10).await;
    }
}
