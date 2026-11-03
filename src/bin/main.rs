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
use embassy_futures::select::{select, Either};
use heapless::Vec;
use keerp::engine::Engine;
use keerp::layer::HoldState::{Hold, Waiting};
use keerp::layer::KeyTouch::{Chord, HoldTap, Key, LayerOn, ModTap};
use keerp::layer::LayerKey::{self};
use keerp::layer::ModifiersKey::{self, LCTRL, LSHIFT};
use keerp::layer::{KeyEvent, KeyTouch};

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::{DMA_CH0, PIO0, UART0, USB};
use embassy_rp::pio::Pio;
use embassy_rp::pio_programs::uart;
use embassy_rp::pio_programs::uart::{PioUartRx, PioUartRxProgram};
use embassy_rp::uart::{Async, Config, Uart, UartRx};
use embassy_rp::usb::{Driver, Out};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{with_timeout, Duration, Instant, Timer};
use embassy_usb::class::hid::{HidBootProtocol, HidProtocolMode, HidSubclass, HidWriter, ReportId};
use embassy_usb::class::hid::{HidReaderWriter, RequestHandler, State};
use embassy_usb::control::OutResponse;
use embassy_usb::{Builder, Handler};
use embedded_io_async::Read;
use panic_halt as _;
use usbd_hid::descriptor::{KeyboardReport, KeyboardUsage, SerializedDescriptor};

use keerp::{engine, matrix};

static SLAVE_EVENTS: Channel<ThreadModeRawMutex, [u8; 5], 64> = Channel::new();

static EVENT_CHANNEL: Channel<ThreadModeRawMutex, KeyEvent, 32> = Channel::new();

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<USB>;
    UART0_IRQ => embassy_rp::uart::InterruptHandler<embassy_rp::peripherals::UART0>;
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<DMA_CH0>;
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
});

/*
┌─────────────────────────────────────────────────┐
│ q w e r t y                                     │      ╭╮╭╮╭╮╭╮
└─────────────────────────────────────────────────┘      │╰╯╰╯╰╯│
          ┌─────────┬─────────┬─────────┬─────────┬──────╨──┐┌──╨──────┬─────────┬─────────┬─────────┬─────────┐
  ╌┄┈┈───═╡    Q    │    W    │    E    │    R    │    T    ││    Y    │    U    │    I    │    O    │    P    │
          ├─────────┼─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┼─────────┤
          │    A    │    S    │    D    │    F    │    G    ││    H    │    J    │    K    │    L    │    ;    │
┌─────────┼─────────┼─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┼─────────┼─────────┐
│    Q    │    Z    │    X    │    C    │    V    │    B    ││    N    │    M    │    ,    │    .    │    /    │    P    │
└─────────┴─────────┴─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┴─────────┴─────────┘
                              │  CTRL   │  LOWER  │  SPACE  ││  ENTER  │  RAISE  │  BSPC   │
                              └─────────┴─────────┴─────────┘└─────────┴─────────┴─────────┘*/

const BASE: [[KeyTouch; 10]; 4] = [
    [
        // Left Top Row (Q, W, F, P, B)
        KeyTouch::Key(KeyboardUsage::KeyboardQq),
        KeyTouch::Key(KeyboardUsage::KeyboardWw),
        KeyTouch::Key(KeyboardUsage::KeyboardFf),
        KeyTouch::Key(KeyboardUsage::KeyboardPp),
        KeyTouch::Key(KeyboardUsage::KeyboardBb),
        // Right Top Row (J, L, U, Y, ')
        KeyTouch::Key(KeyboardUsage::KeyboardJj),
        KeyTouch::Key(KeyboardUsage::KeyboardLl),
        KeyTouch::Key(KeyboardUsage::KeyboardUu),
        KeyTouch::Key(KeyboardUsage::KeyboardYy),
        KeyTouch::Key(KeyboardUsage::KeyboardSingleDoubleQuote),
    ],
    [
        // Left Home Row Mods: GUI (A), ALT (R), CTRL (S), SHIFT (T), Key (G)
        KeyTouch::ModTap(ModifiersKey::LGUI, KeyboardUsage::KeyboardAa),
        KeyTouch::ModTap(ModifiersKey::LALT, KeyboardUsage::KeyboardRr),
        KeyTouch::ModTap(ModifiersKey::LCTRL, KeyboardUsage::KeyboardSs),
        KeyTouch::ModTap(ModifiersKey::LSHIFT, KeyboardUsage::KeyboardTt),
        KeyTouch::Key(KeyboardUsage::KeyboardGg),
        // Right Home Row Mods: Key (M), SHIFT (N), CTRL (E), ALT (I), GUI (O)
        KeyTouch::Key(KeyboardUsage::KeyboardMm),
        KeyTouch::ModTap(ModifiersKey::RSHIFT, KeyboardUsage::KeyboardNn),
        KeyTouch::ModTap(ModifiersKey::RCTRL, KeyboardUsage::KeyboardEe),
        KeyTouch::ModTap(ModifiersKey::RALT, KeyboardUsage::KeyboardIi),
        KeyTouch::ModTap(ModifiersKey::RGUI, KeyboardUsage::KeyboardOo),
    ],
    [
        // Left Bottom Row (Z, X, C, D, V)
        KeyTouch::Key(KeyboardUsage::KeyboardZz),
        KeyTouch::Key(KeyboardUsage::KeyboardXx),
        KeyTouch::Key(KeyboardUsage::KeyboardCc),
        KeyTouch::Key(KeyboardUsage::KeyboardDd),
        KeyTouch::Key(KeyboardUsage::KeyboardVv),
        // Right Bottom Row (K, H, ,, ., /)
        KeyTouch::Key(KeyboardUsage::KeyboardKk),
        KeyTouch::Key(KeyboardUsage::KeyboardHh),
        KeyTouch::Key(KeyboardUsage::KeyboardCommaLess),
        KeyTouch::Key(KeyboardUsage::KeyboardPeriodGreater),
        KeyTouch::Key(KeyboardUsage::KeyboardSlashQuestion),
    ],
    [
        // Left Thumbs (Media, Nav, Mouse)
        KeyTouch::None, // Outer pinky drop
        KeyTouch::None,
        KeyTouch::Key(KeyboardUsage::KeyboardEscape),
        // KeyTouch::HoldTap(LayerKey::Media, KeyboardUsage::KeyboardEscape),
        KeyTouch::HoldTap(LayerKey::Nav, KeyboardUsage::KeyboardSpacebar),
        // KeyTouch::None,
        KeyTouch::Key(KeyboardUsage::KeyboardTab),
        // Right Thumbs (Sym, Num, Fun)
        KeyTouch::HoldTap(LayerKey::Sym, KeyboardUsage::KeyboardEnter),
        KeyTouch::HoldTap(LayerKey::Num, KeyboardUsage::KeyboardBackspace),
        KeyTouch::Key(KeyboardUsage::KeyboardDelete),
        // KeyTouch::HoldTap(LayerKey::Fun, KeyboardUsage::KeyboardDelete),
        KeyTouch::None,
        KeyTouch::None, // Outer pinky drop
    ],
];
/*
╺━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╸

┌─────────────────────────────────────────────────┐
│ l o w e r                                       │      ╭╮╭╮╭╮╭╮
└─────────────────────────────────────────────────┘      │╰╯╰╯╰╯│
          ┌─────────┬─────────┬─────────┬─────────┬──────╨──┐┌──╨──────┬─────────┬─────────┬─────────┬─────────┐
  ╌┄┈┈───═╡ CAPSLCK │ NUMLCK  │    ↑    │    =    │    {    ││    }    │    7    │    8    │    9    │    +    │
          ├─────────┼─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┼─────────┤
          │  HOME   │    ←    │    ↓    │    →    │    [    ││    ]    │    4    │    5    │    6    │    -    │
┌─────────┼─────────┼─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┼─────────┼─────────┐
│ SCRNSHT │   END   │   PG↑   │  SAVE   │   PG↓   │    (    ││    )    │    1    │    2    │    3    │    *    │   ESC   │
└─────────┴─────────┴─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┴─────────┴─────────┘
                              │    ▼    │    ▼    │    ▼    ││    ▼    │ ADJUST  │    0    │
                              └─────────┴─────────┴─────────┘└─────────┴─────────┴─────────┘ */

// ==========================================
// LAYER 1: NAV (Held by Left Middle Thumb)
// ==========================================
pub const NAV: [[KeyTouch; 10]; 4] = [
    [
        // Left Hand (Empty/Disabled while holding thumb)
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        // Right Hand (Editing & Clipboard)
        KeyTouch::Chord(LCTRL, KeyboardUsage::KeyboardYy), //Redo
        KeyTouch::Chord(LCTRL, KeyboardUsage::KeyboardZz), // Undo
        KeyTouch::Chord(LCTRL, KeyboardUsage::KeyboardXx), // Cut
        KeyTouch::Chord(LCTRL, KeyboardUsage::KeyboardCc), // Copy
        KeyTouch::Chord(LCTRL, KeyboardUsage::KeyboardVv), // Paste
    ],
    [
        // Left Hand (Dedicated modifiers for highlighting/jumping words)
        KeyTouch::Key(KeyboardUsage::KeyboardLeftGUI),
        KeyTouch::Key(KeyboardUsage::KeyboardLeftAlt),
        KeyTouch::Key(KeyboardUsage::KeyboardLeftControl),
        KeyTouch::Key(KeyboardUsage::KeyboardLeftShift),
        KeyTouch::None,
        // Right Hand (Arrows on the home row)
        KeyTouch::Key(KeyboardUsage::KeyboardLeftArrow),
        KeyTouch::Key(KeyboardUsage::KeyboardDownArrow),
        KeyTouch::Key(KeyboardUsage::KeyboardUpArrow),
        KeyTouch::Key(KeyboardUsage::KeyboardRightArrow),
        KeyTouch::Key(KeyboardUsage::KeyboardCapsLock),
    ],
    [
        // Left Hand
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        // Right Hand
        KeyTouch::Key(KeyboardUsage::KeyboardInsert),
        KeyTouch::Key(KeyboardUsage::KeyboardHome),
        KeyTouch::Key(KeyboardUsage::KeyboardPageDown),
        KeyTouch::Key(KeyboardUsage::KeyboardPageUp),
        KeyTouch::Key(KeyboardUsage::KeyboardEnd),
    ],
    [
        // Left Thumbs (You are currently holding the middle one to stay on this layer)
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        // Right Thumbs (Convenience keys while navigating)
        KeyTouch::Key(KeyboardUsage::KeyboardEnter),
        KeyTouch::Key(KeyboardUsage::KeyboardBackspace),
        KeyTouch::Key(KeyboardUsage::KeyboardDelete),
        KeyTouch::None,
        KeyTouch::None,
    ],
];
/*
╺━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╸

┌─────────────────────────────────────────────────┐
│ r a i s e                                       │      ╭╮╭╮╭╮╭╮
└─────────────────────────────────────────────────┘      │╰╯╰╯╰╯│
          ┌─────────┬─────────┬─────────┬─────────┬──────╨──┐┌──╨──────┬─────────┬─────────┬─────────┬─────────┐
  ╌┄┈┈───═╡    !    │    @    │    #    │    $    │    %    ││    ^    │    &    │    Ü    │    °    │    /    │
          ├─────────┼─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┼─────────┤
          │    Ä    │    è    │    SZ   │    é    │         ││         │    ¥    │    €    │    £    │    Ö    │
┌─────────┼─────────┼─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┼─────────┼─────────┐
│         │    `    │    ~    │   CUE   │         │         ││         │         │         │ DM REC1 │ DM STOP │ DM PLY1 │
└─────────┴─────────┴─────────┼─────────┼─────────┼─────────┤├─────────┼─────────┼─────────┼─────────┴─────────┴─────────┘
                              │  GIPHY  │ ADJUST  │    ▼    ││    ▼    │    ▼    │    ▼    │
                              └─────────┴─────────┴─────────┘└─────────┴─────────┴─────────┘ */

// ==========================================
// LAYER 2: NUM (Number Pad)
// ==========================================
pub const NUM: [[KeyTouch; 10]; 4] = [
    [
        // Left Hand (Numpad Top Row)
        KeyTouch::Key(KeyboardUsage::KeyboardOpenBracketBrace), // [
        KeyTouch::Key(KeyboardUsage::Keyboard7Ampersand),
        KeyTouch::Key(KeyboardUsage::Keyboard8Asterisk),
        KeyTouch::Key(KeyboardUsage::Keyboard9OpenParens),
        KeyTouch::Key(KeyboardUsage::KeyboardCloseBracketBrace), // ]
        // Right Hand (Disabled while holding thumb)
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
    ],
    [
        // Left Hand (Numpad Home Row)
        KeyTouch::Key(KeyboardUsage::KeyboardSemiColon), // ;
        KeyTouch::Key(KeyboardUsage::Keyboard4Dollar),
        KeyTouch::Key(KeyboardUsage::Keyboard5Percent),
        KeyTouch::Key(KeyboardUsage::Keyboard6Caret),
        KeyTouch::Key(KeyboardUsage::KeyboardEqualPlus), // =
        // Right Hand (Dedicated Modifiers for shortcuts like Alt+F4 or Ctrl+1)
        KeyTouch::None,
        KeyTouch::Key(KeyboardUsage::KeyboardRightShift),
        KeyTouch::Key(KeyboardUsage::KeyboardRightControl),
        KeyTouch::Key(KeyboardUsage::KeyboardRightAlt),
        KeyTouch::Key(KeyboardUsage::KeyboardRightGUI),
    ],
    [
        // Left Hand (Numpad Bottom Row)
        KeyTouch::Key(KeyboardUsage::KeyboardBacktickTilde), // `
        KeyTouch::Key(KeyboardUsage::Keyboard1Exclamation),
        KeyTouch::Key(KeyboardUsage::Keyboard2At),
        KeyTouch::Key(KeyboardUsage::Keyboard3Hash),
        KeyTouch::Key(KeyboardUsage::KeyboardBackslashBar), // \
        // Right Hand
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
    ],
    [
        // Left Thumbs (0, Period, Minus)
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::Key(KeyboardUsage::KeyboardPeriodGreater), // .
        KeyTouch::Key(KeyboardUsage::Keyboard0CloseParens),  // 0
        KeyTouch::Key(KeyboardUsage::KeyboardDashUnderscore), // -
        // Right Thumbs (You are holding the middle one to stay on this layer)
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
    ],
];

// ==========================================
// LAYER 3: SYM (Symbols)
// ==========================================
pub const SYM: [[KeyTouch; 10]; 4] = [
    [
        // Left Hand (Symbols Top Row)
        KeyTouch::Chord(LSHIFT, KeyboardUsage::KeyboardOpenBracketBrace), // {
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard7Ampersand),       // &
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard8Asterisk),        // *
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard9OpenParens),      // (
        KeyTouch::Chord(LSHIFT, KeyboardUsage::KeyboardCloseBracketBrace), // }
        // Right Hand
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
    ],
    [
        // Left Hand (Symbols Home Row)
        KeyTouch::Chord(LSHIFT, KeyboardUsage::KeyboardSemiColon), // :
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard4Dollar),   // $
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard5Percent),  // %
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard6Caret),    // ^
        KeyTouch::Chord(LSHIFT, KeyboardUsage::KeyboardEqualPlus), // +
        // Right Hand (Dedicated Modifiers)
        KeyTouch::None,
        KeyTouch::Key(KeyboardUsage::KeyboardRightShift),
        KeyTouch::Key(KeyboardUsage::KeyboardRightControl),
        KeyTouch::Key(KeyboardUsage::KeyboardRightAlt),
        KeyTouch::Key(KeyboardUsage::KeyboardRightGUI),
    ],
    [
        // Left Hand (Symbols Bottom Row)
        KeyTouch::Chord(LSHIFT, KeyboardUsage::KeyboardBacktickTilde), // ~
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard1Exclamation),  // !
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard2At),           // @
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard3Hash),         // #
        KeyTouch::Chord(LSHIFT, KeyboardUsage::KeyboardBackslashBar),  // |
        // Right Hand
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
    ],
    [
        // Left Thumbs (Parentheses and Underscore)
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard9OpenParens), // (
        KeyTouch::Chord(LSHIFT, KeyboardUsage::Keyboard8Asterisk),   // )
        KeyTouch::Chord(LSHIFT, KeyboardUsage::KeyboardDashUnderscore), // _
        // Right Thumbs (You are holding the inner one to stay on this layer)
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
        KeyTouch::None,
    ],
];

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = Driver::new(p.USB, Irqs);

    // Create embassy-usb Config, USB Device Descriptor
    let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
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
        poll_ms: 20,
        max_packet_size: 64,
        hid_subclass: HidSubclass::Boot,
        hid_boot_protocol: HidBootProtocol::Keyboard,
    };

    let hid = HidReaderWriter::<_, 1, 8>::new(&mut builder, &mut state, config);

    // Build the builder
    let mut usb = builder.build();

    let usb_fut = usb.run();

    let (reader, mut writer) = hid.split();

    let mut led = Output::new(p.PIN_25, Level::Low);

    let mut rows = [
        Output::new(p.PIN_26, Level::High),
        Output::new(p.PIN_27, Level::High),
        Output::new(p.PIN_28, Level::High),
        Output::new(p.PIN_29, Level::High),
    ];

    let mut cols = [
        Input::new(p.PIN_6, Pull::Up),
        Input::new(p.PIN_7, Pull::Up),
        Input::new(p.PIN_3, Pull::Up),
        Input::new(p.PIN_4, Pull::Up),
        Input::new(p.PIN_2, Pull::Up),
    ];

    // Enable schmitt trigger to avoid noise
    for c in &mut cols {
        c.set_schmitt(true);
    }

    let keymap: [[[KeyTouch; 10]; 4]; 4] = [BASE, NAV, NUM, SYM];

    let mut layout = engine::Engine::new(keymap);

    let mut matrix_key = matrix::Matrix::new(cols, rows);

    // let mut hold_key = None;

    let out_fut = async {
        reader.run(false, &mut request_handler).await;
    };

    //UART task
    let mut pio = Pio::new(p.PIO0, Irqs);

    let rx_program = PioUartRxProgram::new(&mut pio.common);

    // Force an RX onto PIN 0
    let uart_rx = PioUartRx::new(115200, &mut pio.common, pio.sm0, p.PIN_0, &rx_program);

    // Run everything concurrently.
    join(
        usb_fut,
        join(
            uart_task(uart_rx, led),
            join(
                matrix_scan_task(&mut matrix_key),
                handler_task(writer, &mut layout),
            ),
        ),
    )
    .await;
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

async fn handler_task<'d, const L: usize>(
    mut writer: HidWriter<'d, Driver<'d, USB>, 8>,
    engine: &mut Engine<L>,
) {
    loop {
        match engine.next_deadline() {
            Some(d) => {
                match select(EVENT_CHANNEL.receive(), Timer::at(Instant::from_millis(d))).await {
                    Either::First(e) => engine.handle(e),
                    Either::Second(_) => engine.on_timeout(Instant::now().as_millis()),
                }
            }
            None => engine.handle(EVENT_CHANNEL.receive().await),
        }
        for report in engine.take_reports() {
            let send_report = KeyboardReport {
                modifier: report.modifiers,
                reserved: 0,
                leds: 0,
                keycodes: report.key_codes,
            };

            let _ = writer.write_serialize(&send_report).await;
        }
        // Timer::after_millis(10).await;
    }
}

async fn matrix_scan_task<'d, const CS: usize, const RS: usize>(
    matrix: &mut matrix::Matrix<'d, CS, RS>,
) {
    let mut master_keys = [[false; 10]; 4];

    loop {
        let left_keys = matrix.scan().await;

        for r in 0..RS {
            for c in 0..5 {
                // CS is 5
                if master_keys[r][c] != left_keys[r][c] {
                    master_keys[r][c] = left_keys[r][c];
                    let events = KeyEvent {
                        row: r,
                        col: c,
                        is_pressed: left_keys[r][c],
                        time_ms: Instant::now().as_millis(),
                    };
                    EVENT_CHANNEL.send(events).await;
                }
            }
        }
    }
}

async fn uart_task<'d>(mut uart_rx: PioUartRx<'static, PIO0, 0>, mut led: Output<'d>) {
    let mut buf = [0u8; 5];
    led.set_low();

    loop {
        if uart_rx.read(&mut buf).await.is_ok() {
            if buf[0] != 0x67 || buf[4] != 0x67 {
                continue;
            }

            let event = KeyEvent {
                row: buf[1] as usize,
                col: buf[2] as usize,
                is_pressed: buf[3] == 1,
                time_ms: Instant::now().as_millis(),
            };

            EVENT_CHANNEL.send(event).await;

            //for testing
            if led.is_set_high() {
                led.set_low();
            } else {
                led.set_high();
            }
        }
    }
}
