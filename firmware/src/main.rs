//! Прошивка наземной станции для LILYGO T-Beam (ESP32).
//!
//! Плата работает прозрачным мостом между компьютером и эфиром:
//!
//! ```text
//! ПК ──USB (UART0, CH9102)──► ESP32 ──SPI──► LoRa-чип ──► эфир
//! ПК ◄─USB──────────────────── ESP32 ◄─SPI── LoRa-чип ◄── эфир
//! ```
//!
//! Радио обслуживается драйверами из `sat-drivers` (репозиторий lorasat):
//! `Radio1262` для SX1262 и `Radio` для SX1276. Оба реализуют `PhyLayer`.
//! Кадры канального и транспортного уровней формирует программа на ПК,
//! прошивка их не разбирает: какие байты пришли по USB, такие и уходят в эфир.
//!
//! Формат на USB: `[COBS(пакет)][0x00]` (см. `usb.rs`).

#![no_std]
#![no_main]

mod power;
mod radio;
mod usb;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_backtrace as _;
use esp_hal::i2c::master::I2c;
use esp_hal::spi::master::Spi;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{self, Uart, UartRx};
use esp_hal::Async;
use sat_core::comm::phy::PhyLayer;

use crate::usb::{Packet, UsbDecoder};

esp_bootloader_esp_idf::esp_app_desc!();


const LORA_FREQ_HZ: u32 = 868_000_000;


const USB_BAUD: u32 = 115_200;

/// Пакеты с ПК ожидающие передачи в эфир
static USB_TO_RADIO: Channel<CriticalSectionRawMutex, Packet, 4> = Channel::new();


#[defmt::global_logger]
struct NullLogger;

unsafe impl defmt::Logger for NullLogger {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // --- USB-Serial (UART0: TX = GPIO1, RX = GPIO3) ---
    let uart = Uart::new(p.UART0, uart::Config::default().with_baudrate(USB_BAUD))
        .unwrap()
        .with_tx(p.GPIO1)
        .with_rx(p.GPIO3)
        .into_async();
    let (usb_rx, mut usb_tx) = uart.split();

    // --- Питание радиочипа через PMU (I2C: SDA = GPIO21, SCL = GPIO22) ---
    let mut i2c = I2c::new(p.I2C0, esp_hal::i2c::master::Config::default())
        .unwrap()
        .with_sda(p.GPIO21)
        .with_scl(p.GPIO22);
    let pmu = power::enable_lora_power(&mut i2c);

    // --- SPI к радиочипу: SCK = 5, MISO = 19, MOSI = 27, NSS = 18 ---
    let spi = Spi::new(
        p.SPI2,
        esp_hal::spi::master::Config::default()
            .with_frequency(Rate::from_khz(2000))
            .with_mode(esp_hal::spi::Mode::_0),
    )
    .unwrap()
    .with_sck(p.GPIO5)
    .with_miso(p.GPIO19)
    .with_mosi(p.GPIO27);

    let pins = radio::RadioPins {
        nss: p.GPIO18,
        reset: p.GPIO23,
        dio0: p.GPIO26,
        dio1: p.GPIO33,
        busy: p.GPIO32,
    };

    let mut radio = match radio::init(spi, pins, LORA_FREQ_HZ).await {
        Ok(r) => r,
        Err(e) => {
            // Сообщение увидит `espflash monitor`; программа на ПК его отбросит.
            usb::write_text(&mut usb_tx, "gs-bridge: radio init failed: ").await;
            usb::write_text(&mut usb_tx, e).await;
            usb::write_text(&mut usb_tx, "\r\n").await;
            panic!("radio init failed: {}", e);
        }
    };

    usb::write_text(&mut usb_tx, "gs-bridge: pmu=").await;
    usb::write_text(&mut usb_tx, pmu.name()).await;
    usb::write_text(&mut usb_tx, " radio=").await;
    usb::write_text(&mut usb_tx, radio.chip_name()).await;
    usb::write_text(&mut usb_tx, " ready\r\n").await;
    // Нулевой байт закрывает текст как отдельный «пакет»: на ПК он не
    // декодируется и отбрасывается, следующий пакет придёт чистым.
    usb::write_raw(&mut usb_tx, &[0x00]).await;

    spawner.spawn(usb_rx_task(usb_rx).unwrap());

    let mut rx_buf = [0u8; 255];
    loop {
        match select(USB_TO_RADIO.receive(), radio.recv_bytes(&mut rx_buf)).await {
            // ПК → эфир
            Either::First(pkt) => {
                match radio.send_bytes(&pkt).await {
                    //let _ = radio.send_bytes(&pkt).await;
                    Ok(()) => usb::write_text(&mut usb_tx, "gs-bridge: tx ok\r\n").await,
                    Err(_) => usb::write_text(&mut usb_tx, "gs-bridge: tx error\r\n").await,
                }
                usb::write_raw(&mut usb_tx, &[0x00]).await;
            }
            // эфир → ПК
            Either::Second(Ok(n)) => {
                usb::write_packet(&mut usb_tx, &rx_buf[..n]).await;
            }
            Either::Second(Err(_)) => {}
        }
    }
}

/// Читает поток байт с USB, собирает пакеты по разделителю 0x00 и кладёт их в очередь на передачу в эфир
#[embassy_executor::task]
async fn usb_rx_task(mut rx: UartRx<'static, Async>) {
    let mut decoder = UsbDecoder::new();
    let mut chunk = [0u8; 64];
    loop {
        let n = match rx.read_async(&mut chunk).await {
            Ok(n) => n,
            Err(_) => continue,
        };
        for &b in &chunk[..n] {
            if let Some(pkt) = decoder.push(b) {
                USB_TO_RADIO.send(pkt).await;
            }
        }
    }
}
