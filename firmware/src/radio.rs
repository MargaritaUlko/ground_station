//! Радиочип T-Beam через драйверы `sat-drivers`.
//!
//! На T-Beam ставят либо SX1262, либо SX1276 (зависит от партии). Пины у них
//! разные, драйверы в `sat-drivers` тоже разные:
//!
//! | Сигнал | SX1262 (`Radio1262`) | SX1276 (`Radio`) |
//! |--------|----------------------|------------------|
//! | NSS    | GPIO18               | GPIO18           |
//! | RESET  | GPIO23               | GPIO23           |
//! | IRQ    | DIO1 = GPIO33        | DIO0 = GPIO26    |
//! | BUSY   | GPIO32               | —                |
//!
//! Чип определяется до создания драйвера: после сброса читается регистр
//! версии SX127x (0x42). SX1276 отвечает 0x12, SX1262 отвечает байтом статуса.

use embassy_time::Delay;
use embedded_hal::spi::SpiBus;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::peripherals::{GPIO18, GPIO23, GPIO26, GPIO32, GPIO33};
use esp_hal::spi::master::Spi;
use esp_hal::{Async, Blocking};
use sat_core::comm::phy::PhyLayer;
use sat_drivers::lora::{Error as LoraError, Radio, Radio1262};

type SpiDev = ExclusiveDevice<Spi<'static, Async>, Output<'static>, Delay>;
type Ctrl = Output<'static>;
type Wait = Input<'static>;

pub struct RadioPins {
    pub nss: GPIO18<'static>,
    pub reset: GPIO23<'static>,
    pub dio0: GPIO26<'static>,
    pub dio1: GPIO33<'static>,
    pub busy: GPIO32<'static>,
}

pub enum AnyRadio {
    Sx1262(Radio1262<SpiDev, Ctrl, Wait, Delay>),
    Sx1276(Radio<SpiDev, Ctrl, Wait, Delay>),
}

impl AnyRadio {
    pub fn chip_name(&self) -> &'static str {
        match self {
            AnyRadio::Sx1262(_) => "SX1262",
            AnyRadio::Sx1276(_) => "SX1276",
        }
    }
}

impl PhyLayer for AnyRadio {
    type Error = LoraError;

    async fn send_bytes(&mut self, payload: &[u8]) -> Result<(), Self::Error> {
        match self {
            AnyRadio::Sx1262(r) => r.send_bytes(payload).await,
            AnyRadio::Sx1276(r) => r.send_bytes(payload).await,
        }
    }

    async fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            AnyRadio::Sx1262(r) => r.recv_bytes(buf).await,
            AnyRadio::Sx1276(r) => r.recv_bytes(buf).await,
        }
    }
}

const SX127X_REG_VERSION: u8 = 0x42;
const SX1276_VERSION: u8 = 0x12;


fn detect_sx1276(spi: &mut Spi<'static, Blocking>, nss: &mut Output<'static>, reset: &mut Output<'static>) -> bool {
    let delay = esp_hal::delay::Delay::new();
    reset.set_low();
    delay.delay_millis(10);
    reset.set_high();
    delay.delay_millis(20);

    let mut buf = [SX127X_REG_VERSION & 0x7F, 0x00]; // старший бит 0 = чтение
    nss.set_low();
    let ok = spi.transfer_in_place(&mut buf).is_ok();
    nss.set_high();
    ok && buf[1] == SX1276_VERSION
}

pub async fn init(
    mut spi: Spi<'static, Blocking>,
    pins: RadioPins,
    freq_hz: u32,
) -> Result<AnyRadio, &'static str> {
    let mut nss = Output::new(pins.nss, Level::High, OutputConfig::default());
    let mut reset = Output::new(pins.reset, Level::High, OutputConfig::default());

    let is_sx1276 = detect_sx1276(&mut spi, &mut nss, &mut reset);

    let spi = ExclusiveDevice::new(spi.into_async(), nss, Delay).map_err(|_| "spi device")?;
    let in_cfg = InputConfig::default().with_pull(Pull::Up);

    if is_sx1276 {
        let dio0 = Input::new(pins.dio0, in_cfg);
        Radio::new(spi, reset, dio0, Delay, freq_hz)
            .await
            .map(AnyRadio::Sx1276)
            .map_err(|_| "SX1276 init")
    } else {
        let dio1 = Input::new(pins.dio1, in_cfg);
        let busy = Input::new(pins.busy, in_cfg);
        Radio1262::new(spi, reset, dio1, busy, None, None, Delay, freq_hz)
            .await
            .map(AnyRadio::Sx1262)
            .map_err(|_| "SX1262 init (chip not responding: check PMU power and antenna)")
    }
}
