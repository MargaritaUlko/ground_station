//! PhyLayer через USB-Serial: адаптер «ПК ↔ плата-мост ↔ LoRa».
//!
//! Программа на ПК пишет байты в COM-порт, прошивка T-Beam (`firmware/`) принимает их
//! по USB и передаёт в радиочип по SPI, радиочип отправляет их в эфир.
//! Пакет, принятый платой из эфира, приходит обратно в COM-порт.
//!
//! # Формат на USB
//!
//! Каждый пакет кодируется COBS и завершается байтом 0x00:
//!
//! ```text
//! [COBS(payload)][0x00]
//! ```
//!
//! После COBS внутри пакета нет нулевых байтов, поэтому 0x00
//! однозначно обозначает конец пакета. Тот же разделитель используется
//! в `sat_drivers::uart::Uart::recv_bytes` и в прошивке `firmware/src/usb.rs`.






// 1  use core::fmt::Debug;
// 2
// 3  #[allow(async_fn_in_trait)]
// 4  pub trait PhyLayer {
// 5      type Error: Debug;
// 6
// 7      async fn send_bytes(&mut self, payload: &[u8]) -> Result<(), Self::Error>;
// 8      async fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
// 9  }
use std::io;
use std::time::Duration;


use sat_core::comm::phy::PhyLayer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialStream};



pub const BAUD_RATE: u32 = 115_200;

const MAX_ENCODED_LEN: usize = 1024;

pub struct PhySerial {
    port: SerialStream,
    rx: Vec<u8>,
}
// определяем метод класса PhySerial
impl PhySerial {
    pub async fn new(path: &str) -> io::Result<Self> {
        let port = tokio_serial::new(path, BAUD_RATE)
            .open_native_async()
            .map_err(io::Error::other)?;

        tokio::time::sleep(Duration::from_millis(2000)).await;
        port.clear(tokio_serial::ClearBuffer::Input)
            .map_err(io::Error::other)?;

        Ok(Self {
            port,
            rx: Vec::with_capacity(MAX_ENCODED_LEN),
        })
    }
}

impl PhyLayer for PhySerial {
    type Error = io::Error;

    async fn send_bytes(&mut self, payload: &[u8]) -> Result<(), Self::Error> {
        let mut frame = cobs_encode(payload);
        frame.push(0x00);
        self.port.write_all(&frame).await?;
        self.port.flush().await?;
        log::debug!("serial tx {} bytes", payload.len());
        Ok(())
    }

    async fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> { //получение через юсб
        let mut byte = [0u8; 1];
        loop {
                match self.port.read_exact(&mut byte).await {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
                Err(e) => return Err(e),
            }

            if byte[0] != 0x00 {
                self.rx.push(byte[0]);
                if self.rx.len() > MAX_ENCODED_LEN {
                    self.rx.clear();
                }
                continue;
            }

            // Разделитель: пакет закончен.
            let encoded = std::mem::take(&mut self.rx);
            if encoded.is_empty() {
                continue;
            }
            match cobs_decode(&encoded, buf) {
                Some(n) if n > 0 => {
                    log::debug!("serial rx {} bytes", n);
                    return Ok(n);
                }
                // _ => log::warn!("serial: битый пакет ({} байт), пропущен", encoded.len()),
                _ => match core::str::from_utf8(&encoded) {
                    Ok(text) => log::info!("плата: {}", text.trim_end()),
                    Err(_) => log::warn!("serial: битый пакет ({} байт), пропущен", encoded.len()),
                },
            }
        }
    }
}

/// COBS-кодирование: убирает из данных все байты 0x00.
pub fn cobs_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 254 + 2);
    let mut code_idx = 0;
    out.push(0);
    let mut code: u8 = 1;

    for &b in data {
        if b == 0 {
            out[code_idx] = code;
            code_idx = out.len();
            out.push(0);
            code = 1;
        } else {
            out.push(b);
            code += 1;
            if code == 0xFF {
                out[code_idx] = code;
                code_idx = out.len();
                out.push(0);
                code = 1;
            }
        }
    }
    out[code_idx] = code;
    out
}


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

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let enc = cobs_encode(data);
        assert!(!enc.contains(&0), "в COBS не должно быть нулей");
        let mut dec = vec![0u8; data.len() + 8];
        let n = cobs_decode(&enc, &mut dec).unwrap();
        assert_eq!(&dec[..n], data);
    }

    #[test]
    fn cobs_roundtrip() {
        roundtrip(b"");
        roundtrip(b"ping");
        roundtrip(&[0]);
        roundtrip(&[0, 0, 1, 0]);
        roundtrip(&[1, 2, 3, 0, 4, 5, 0]);
        let long: Vec<u8> = (0..600u32).map(|i| (i % 256) as u8).collect();
        roundtrip(&long);
        let no_zero: Vec<u8> = (0..300u32).map(|i| (i % 255 + 1) as u8).collect();
        roundtrip(&no_zero);
    }
}
