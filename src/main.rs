//! Blink the XIAO RP2040 / RP2350's onboard user LED with `embassy-rp` + the embassy
//! executor (`--framework embassy`).
//!
//! Source: embassy-rs/embassy `examples/rp/src/bin/blinky.rs`
//! (provenance ef19efcdb3a8948a607a0e1852ed74d4323fa11f) with the pin taken
//! from the registry and defmt logging removed. Compile-verified:
//! research/hal-framework-matrix.md cell 8.
#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::Timer;
use embassy_usb::class::hid::{HidBootProtocol, HidProtocolMode, HidSubclass, HidWriter, ReportId};
use embassy_usb::class::hid::{HidReaderWriter, RequestHandler, State};
use embassy_usb::control::OutResponse;
use embassy_usb::{Builder, Config, Handler};
use panic_halt as _;
use usbd_hid::descriptor::{KeyboardReport, KeyboardUsage, SerializedDescriptor};

use crate::matrix::Matrix;
pub mod matrix;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = Driver::new(p.USB, Irqs);

    // Create embassy-usb Config, USB Device Descriptor
    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("Embassy");
    config.product = Some("HID keyboard example");
    config.serial_number = Some("12345678");
    config.max_power = 100;
    config.max_packet_size_0 = 64;
    config.composite_with_iads = false;
    config.device_class = 0;
    config.device_sub_class = 0;
    config.device_protocol = 0;

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    // You can also add a Microsoft OS descriptor.
    let mut msos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut request_handler = MyRequestHandler {};
    let mut device_handler = MyDeviceHandler::new();

    let mut state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buf,
    );

    // Classes on the builder
    let config = embassy_usb::class::hid::Config {
        report_descriptor: KeyboardReport::desc(),
        request_handler: None,
        poll_ms: 60,
        max_packet_size: 64,
        hid_subclass: HidSubclass::Boot,
        hid_boot_protocol: HidBootProtocol::Keyboard,
    };

    let hid = HidReaderWriter::<_, 1, 8>::new(&mut builder, &mut state, config);

    // Build the builder
    let mut usb = builder.build();

    let usb_fut = usb.run();

    // TODO: Test with different PIN here
    // let mut r1 = Output::new(p.PIN_29, Level::High);
    // let mut c1 = Input::new(p.PIN_2, Pull::Up);
    // let mut c2 = Input::new(p.PIN_4, Pull::Up);

    let (reader, mut writer) = hid.split();

    let mut led = Output::new(p.PIN_25, Level::Low);

    let mut rows = [
        Output::new(p.PIN_26, Level::High),
        Output::new(p.PIN_27, Level::High),
        Output::new(p.PIN_28, Level::High),
        Output::new(p.PIN_29, Level::High),
    ];

    // 5 Columns initialized directly into the array as Inputs
    let mut cols = [
        Input::new(p.PIN_2, Pull::Up),
        Input::new(p.PIN_4, Pull::Up),
        Input::new(p.PIN_3, Pull::Up),
        Input::new(p.PIN_7, Pull::Up),
        Input::new(p.PIN_6, Pull::Up),
    ];

    // Enable schmitt trigger to avoid noise
    for c in &mut cols {
        c.set_schmitt(true);
    }

    let keymap: [[KeyboardUsage; 5]; 4] = [
        // Row 0 (Top): Tab, Q, W, E, R
        [
            KeyboardUsage::KeyboardTab,
            KeyboardUsage::KeyboardQq,
            KeyboardUsage::KeyboardWw,
            KeyboardUsage::KeyboardEe,
            KeyboardUsage::KeyboardRr,
        ],
        // Row 1 (Home): Esc (or Caps Lock), A, S, D, F
        [
            KeyboardUsage::KeyboardEscape,
            KeyboardUsage::KeyboardAa,
            KeyboardUsage::KeyboardSs,
            KeyboardUsage::KeyboardDd,
            KeyboardUsage::KeyboardFf,
        ],
        // Row 2 (Bottom): Shift, Z, X, C, V
        [
            KeyboardUsage::KeyboardLeftShift,
            KeyboardUsage::KeyboardZz,
            KeyboardUsage::KeyboardXx,
            KeyboardUsage::KeyboardCc,
            KeyboardUsage::KeyboardVv,
        ],
        // Row 3 (Thumb Cluster): Ctrl, GUI (Windows/Cmd), Alt, Space, B
        [
            KeyboardUsage::KeyboardLeftControl,
            KeyboardUsage::KeyboardLeftGUI,
            KeyboardUsage::KeyboardLeftAlt,
            KeyboardUsage::KeyboardSpacebar,
            KeyboardUsage::KeyboardBb,
        ],
    ];

    let mut matrix_key = matrix::Matrix::new(cols, rows);

    // let mut prev_keys = [[false; 5]; 4];
    // let mut keys = [[false; 5]; 4];
    //
    // let mut key_codes = [0, 0, 0, 0, 0, 0];
    // let mut modifiers = 0;
    // let mut keycode = 0;
    //
    //
    // // Do stuff with the class!
    // let in_fut = async {
    //     loop {
    //         keys = matrix_key.scan().await;
    //
    //         if prev_keys != keys {
    //             key_codes = [0, 0, 0, 0, 0, 0];
    //             modifiers = 0;
    //             let mut index = 0;
    //             for i in 0..4 {
    //                 for j in 0..5 {
    //                     if keys[i][j] {
    //                         keycode = keymap[i][j] as u8;
    //                         if (224..=231).contains(&keycode) {
    //                             modifiers = keycode - 224;
    //                         } else {
    //                             key_codes[index] = keycode;
    //                         }
    //                         index += 1;
    //                     }
    //                 }
    //             }
    //         }
    //
    //         // if matrix_key.scan_a_test().await {
    //         //     led.set_high();
    //         // } else {
    //         //     led.set_low();
    //         // }
    //
    //         let report = KeyboardReport {
    //             modifier: modifiers,
    //             reserved: 0,
    //             leds: 0,
    //             keycodes: key_codes,
    //         };
    //         // Send the report.
    //         match writer.write_serialize(&report).await {
    //             Ok(()) => {}
    //             Err(e) => {}
    //         };
    //
    //         // for r in
    //         // r1.set_low();
    //         //
    //         // for i in 0..2 {
    //         //     if cols[i].is_low() {
    //         //         matrix_key[i] = true;
    //         //     } else {
    //         //         matrix_key[i] = false;
    //         //     }
    //         // }
    //         //
    //         // for i in 0..2 {
    //         //     if matrix_key[i] && !prev_key[i] {
    //         //         let report = KeyboardReport {
    //         //             modifier: 0,
    //         //             reserved: 0,
    //         //             leds: 0,
    //         //             keycodes: [i as u8 + 4, 0, 0, 0, 0, 0],
    //         //         };
    //         //         // Send the report.
    //         //         match writer.write_serialize(&report).await {
    //         //             Ok(()) => {}
    //         //             Err(e) => {}
    //         //         };
    //         //     } else if !matrix_key[i] && prev_key[i] {
    //         //         let report = KeyboardReport {
    //         //             modifier: 0,
    //         //             reserved: 0,
    //         //             leds: 0,
    //         //             keycodes: [0, 0, 0, 0, 0, 0],
    //         //         };
    //         //         match writer.write_serialize(&report).await {
    //         //             Ok(()) => {}
    //         //             Err(e) => {}
    //         //         };
    //         //     }
    //         // }
    //
    //         prev_keys = keys;
    //
    //         // Create a report with the A key pressed. (no shift modifier)
    //
    //         // signal_pin.wait_for_high().await;
    //     }
    // };

    let out_fut = async {
        reader.run(false, &mut request_handler).await;
    };

    // Run everything concurrently.
    // If we had made everything `'static` above instead, we could do this using separate tasks instead.
    join(
        usb_fut,
        join(custom_task(writer, &mut matrix_key, &keymap), out_fut),
    )
    .await;

    // // XIAO onboard user LED (GPIO from the xiao-generate registry).
    // let mut led = Output::new(p.PIN_25, Level::High);
    //
    // loop {
    //     led.set_high();
    //     Timer::after_millis(5000).await;
    //     led.set_low();
    //     Timer::after_millis(5000).await;
    // }
}

struct MyRequestHandler {}

impl RequestHandler for MyRequestHandler {
    fn get_report(&mut self, id: ReportId, _buf: &mut [u8]) -> Option<usize> {
        // info!("Get report for {:?}", id);
        None
    }

    fn set_report(&mut self, id: ReportId, data: &[u8]) -> OutResponse {
        // info!("Set report for {:?}: {=[u8]}", id, data);
        OutResponse::Accepted
    }

    // fn get_protocol(&self) -> HidProtocolMode {
    //     let protocol = HidProtocolMode::from(HID_PROTOCOL_MODE.load(Ordering::Relaxed));
    //     // info!("The current HID protocol mode is: {}", protocol);
    //     protocol
    // }

    // fn set_protocol(&mut self, protocol: HidProtocolMode) -> OutResponse {
    //     // info!("Switching to HID protocol mode: {}", protocol);
    //     HID_PROTOCOL_MODE.store(protocol as u8, Ordering::Relaxed);
    //     OutResponse::Accepted
    // }

    fn set_idle_ms(&mut self, id: Option<ReportId>, dur: u32) {
        // info!("Set idle rate for {:?} to {:?}", id, dur);
    }

    fn get_idle_ms(&mut self, id: Option<ReportId>) -> Option<u32> {
        // info!("Get idle rate for {:?}", id);
        None
    }
}

struct MyDeviceHandler {
    configured: AtomicBool,
}

impl MyDeviceHandler {
    fn new() -> Self {
        MyDeviceHandler {
            configured: AtomicBool::new(false),
        }
    }
}

impl Handler for MyDeviceHandler {
    fn enabled(&mut self, enabled: bool) {
        self.configured.store(false, Ordering::Relaxed);
        // if enabled {
        //     info!("Device enabled");
        // } else {
        //     info!("Device disabled");
        // }
    }

    fn reset(&mut self) {
        self.configured.store(false, Ordering::Relaxed);
        // info!("Bus reset, the Vbus current limit is 100mA");
    }

    fn addressed(&mut self, addr: u8) {
        self.configured.store(false, Ordering::Relaxed);
        // info!("USB address set to: {}", addr);
    }

    fn configured(&mut self, configured: bool) {
        self.configured.store(configured, Ordering::Relaxed);
        // if configured {
        //     info!(
        //         "Device configured, it may now draw up to the configured current limit from Vbus."
        //     )
        // } else {
        //     info!("Device is no longer configured, the Vbus current limit is 100mA.");
        // }
    }
}

async fn custom_task<'d, const CS: usize, const RS: usize>(
    mut writer: HidWriter<'d, Driver<'d, USB>, 8>,
    matrix: &mut Matrix<'d, CS, RS>,
    keymap: &[[KeyboardUsage; CS]; RS],
) {
    let mut prev_keys = [[false; CS]; RS];
    let mut keys;
    let mut key_codes = [0, 0, 0, 0, 0, 0];
    let mut modifiers = 0;
    let mut keycode;
    loop {
        keys = matrix.scan().await;

        if prev_keys != keys {
            key_codes = [0, 0, 0, 0, 0, 0];
            modifiers = 0;
            let mut index = 0;
            for i in 0..4 {
                for j in 0..5 {
                    if keys[i][j] {
                        keycode = keymap[i][j] as u8;
                        if (224..=231).contains(&keycode) {
                            modifiers = keycode - 224;
                        } else {
                            key_codes[index] = keycode;
                        }
                        index += 1;
                    }
                }
            }
        }

        // if matrix_key.scan_a_test().await {
        //     led.set_high();
        // } else {
        //     led.set_low();
        // }

        let report = KeyboardReport {
            modifier: modifiers,
            reserved: 0,
            leds: 0,
            keycodes: key_codes,
        };
        // Send the report.
        match writer.write_serialize(&report).await {
            Ok(()) => {}
            Err(e) => {}
        };

        prev_keys = keys;
    }
}
