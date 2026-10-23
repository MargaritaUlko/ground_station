//! Кадрирование пакетов на USB-Serial.
//!
//! По UART байты идут сплошным потоком, поэтому каждый пакет кодируется COBS
//! и завершается байтом 0x00:
//!
//! ```text
//! [COBS(пакет)][0x00]
//! ```
//!
//! COBS убирает из данных все нулевые байты, поэтому 0x00 однозначно
//! обозначает конец пакета. Если поток сбился (например, при подключении
//! к ПК пришёл обрывок), приёмник восстанавливается на следующем 0x00.
//! Программа на ПК (`host/src/phy_serial.rs`) использует тот же формат.

use esp_hal::Async;
use esp_hal::uart::UartTx;
use heapless::Vec;

/// Максимальный размер пакета LoRa (ограничение SX126x/SX127x).
pub const MAX_PACKET: usize = 255;

/// Максимальный размер закодированного пакета: +1 байт на каждые 254 байта данных и +1.
const MAX_ENCODED: usize = MAX_PACKET + MAX_PACKET / 254 + 2;

pub type Packet = Vec<u8, MAX_PACKET>;

/// Собирает пакеты из потока байт.
pub struct UsbDecoder {
    buf: Vec<u8, MAX_ENCODED>,
    overflow: bool,
}

impl UsbDecoder {
    pub const fn new() -> Self {
        Self {
            buf: Vec::new(),
            overflow: false,
        }
    }

    /// Добавляет байт. Возвращает пакет, когда пришёл разделитель 0x00
    /// и накопленные байты корректно декодировались.
    pub fn push(&mut self, byte: u8) -> Option<Packet> {
        if byte != 0x00 {
            if self.buf.push(byte).is_err() {
                // Слишком длинный кадр: отбрасываем всё до следующего 0x00.
                self.overflow = true;
            }
            return None;
        }

        let result = if self.overflow || self.buf.is_empty() {
            None
        } else {
            let mut out = [0u8; MAX_PACKET];
            match cobs_decode(&self.buf, &mut out) {
                Some(n) if n > 0 => Vec::from_slice(&out[..n]).ok(),
                _ => None,
            }
        };
        self.buf.clear();
        self.overflow = false;
        result
    }
}

/// Отправляет пакет на ПК: `[COBS(data)][0x00]`.
pub async fn write_packet(tx: &mut UartTx<'static, Async>, data: &[u8]) {
    let mut enc = [0u8; MAX_ENCODED + 1];
    let n = cobs_encode(data, &mut enc);
    enc[n] = 0x00;
    write_raw(tx, &enc[..n + 1]).await;
}

/// Отправляет текстовую строку (только для служебных сообщений при загрузке).
pub async fn write_text(tx: &mut UartTx<'static, Async>, s: &str) {
    write_raw(tx, s.as_bytes()).await;
}

pub async fn write_raw(tx: &mut UartTx<'static, Async>, mut data: &[u8]) {
    while !data.is_empty() {
        match tx.write_async(data).await {
            Ok(0) | Err(_) => return,
            Ok(n) => data = &data[n..],
        }
    }
    let _ = tx.flush_async().await;
}

/// COBS-кодирование. Возвращает длину результата в `out` (без завершающего 0x00).
/// `out` должен быть не короче `data.len() + data.len() / 254 + 1`.
pub fn cobs_encode(data: &[u8], out: &mut [u8]) -> usize {
    let mut code_idx = 0;
    let mut o = 1;
    let mut code: u8 = 1;

    for &b in data {
        if b == 0 {
            out[code_idx] = code;
            code_idx = o;
            o += 1;
            code = 1;
        } else {
            out[o] = b;
            o += 1;
            code += 1;
            if code == 0xFF {
                out[code_idx] = code;
                code_idx = o;
                o += 1;
                code = 1;
            }
        }
    }
    out[code_idx] = code;
    o
}

/// COBS-декодирование. `src` без завершающего 0x00.
/// Возвращает длину данных в `dst` или `None`, если кадр повреждён.
pub fn cobs_decode(src: &[u8], dst: &mut [u8]) -> Option<usize> {
    let mut i = 0;
    let mut o = 0;
    while i < src.len() {
        let code = src[i] as usize;
        if code == 0 {
            return None;
        }
        i += 1;
        for _ in 1..code {
            if i >= src.len() || o >= dst.len() {
                return None;
            }
            dst[o] = src[i];
            o += 1;
            i += 1;
        }
        if code < 0xFF && i < src.len() {
            if o >= dst.len() {
                return None;
            }
            dst[o] = 0;
            o += 1;
        }
    }
    Some(o)
}
