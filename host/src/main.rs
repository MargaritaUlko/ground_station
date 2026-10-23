//! Программа наземной станции на ПК.
//!
//! Работает с эфиром через плату T-Beam с прошивкой `firmware/`.
//! Протокол — из репозитория lorasat: `LinkImpl` (канальный уровень)
//! и `TransportImpl` (транспортный уровень) поверх `PhySerial`.
//!
//! ```text
//! gs-host COM5                 — станция: маяки, ping, fetch
//! gs-host COM5 --sniff         — анализатор: печатает все кадры в эфире
//! gs-host COM5 --addr 2 --sat 1
//! ```

mod defmt_stub;
mod phy_serial;

use std::io;
use std::time::Duration;

use log::*;
use sat_core::comm::address::Address;
use sat_core::comm::phy::PhyLayer;
use sat_core::comm::{TransportEvent, TransportLayer, TransportSession};
use sat_core::entity::{Beacon, ClientData};
use sat_drivers::proto_impl::link::LinkImpl;
use sat_drivers::proto_impl::link::frame::Frame;
use sat_drivers::proto_impl::transport::TransportImpl;

use phy_serial::PhySerial;

struct Args {
    port: String,
    gs_addr: Address,
    sat_addr: Address,
    sniff: bool,
}

const USAGE: &str = "usage: gs-host <PORT> [--addr N] [--sat N] [--sniff]\n\
                     \n\
                     PORT     COM-порт платы T-Beam (Windows: COM5, Linux: /dev/ttyUSB0)\n\
                     --addr   адрес станции (по умолчанию 2)\n\
                     --sat    адрес спутника (по умолчанию 1, как в tx_flash)\n\
                     --sniff  печатать все кадры в эфире без фильтра по адресу";

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut args = Args {
        port: String::new(),
        gs_addr: Address(2),
        sat_addr: Address(1),
        sniff: false,
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--addr" => args.gs_addr = Address(parse_u32(it.next())?),
            "--sat" => args.sat_addr = Address(parse_u32(it.next())?),
            "--sniff" => args.sniff = true,
            "-h" | "--help" => return Err(USAGE.into()),
            p if args.port.is_empty() && !p.starts_with('-') => args.port = p.into(),
            other => return Err(format!("неизвестный аргумент: {other}\n\n{USAGE}")),
        }
    }
    if args.port.is_empty() {
        return Err(USAGE.into());
    }
    Ok(args)
}

/// Число в десятичном виде или с префиксом 0x.
fn parse_u32(s: Option<String>) -> Result<u32, String> {
    let s = s.ok_or("после флага ожидается число")?;
    let r = match s.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("не число: {s}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };

    info!("Открываю {} ...", args.port);
    let phy = PhySerial::new(&args.port).await?;

    if args.sniff {
        sniff(phy).await
    } else {
        ground_station(phy, args.gs_addr, args.sat_addr).await
    }
}

/// Режим анализатора: каждый принятый кадр разбирается и печатается.
async fn sniff(mut phy: PhySerial) -> io::Result<()> {
    info!("Режим анализатора: печатаю все кадры (Ctrl+C — выход)");
    let mut buf = [0u8; 255];
    loop {
        let n = phy.recv_bytes(&mut buf).await?;
        let raw = &buf[..n];
        match postcard::from_bytes::<Frame>(raw) {
            Ok(Frame::Beacon(b)) => match postcard::from_bytes::<Beacon>(b.payload) {
                Ok(beacon) => info!(
                    "BEACON  sat={:?} interval={}s timestamp={}",
                    beacon.sat_addr, beacon.interval_sec, beacon.timestamp
                ),
                Err(_) => info!("BEACON  payload={:02x?}", b.payload),
            },
            Ok(Frame::Data(d)) => {
                let what = describe_payload(d.payload);
                info!(
                    "DATA    src=0x{:08x} dst=0x{:08x} len={} {}",
                    d.src,
                    d.dst,
                    d.payload.len(),
                    what
                );
            }
            Err(_) => info!("RAW     {} байт (не кадр lorasat): {:02x?}", n, raw),
        }
    }
}

/// Пытается понять, что лежит в Data-кадре.
fn describe_payload(p: &[u8]) -> String {
    // Транспортный уровень: первый байт — тег (0x00 датаграмма, 0x01 сервисный PDU).
    match p.first() {
        Some(0x00) => {
            return format!("transport datagram: {:?}", String::from_utf8_lossy(&p[1..]));
        }
        Some(0x01) => return format!("transport srv-pdu: {:02x?}", &p[1..]),
        _ => {}
    }
    // Данные клиента без транспорта (client → tx_flash).
    if let Ok(cd) = postcard::from_bytes::<ClientData>(p) {
        return format!("ClientData: {:02x?}", cd.data);
    }
    format!("{:02x?}", p)
}

/// Режим станции: тот же стек, что у `sandbox-pc/src/bin/gs`, но с настоящим радио.
async fn ground_station(phy: PhySerial, gs_addr: Address, sat_addr: Address) -> io::Result<()> {
    let link = LinkImpl::new(gs_addr, phy);
    let mut transport = TransportImpl::new(link);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let cmd = line.trim().to_string();
                    if !cmd.is_empty() && tx.send(cmd).is_err() {
                        break;
                    }
                }
            }
        }
    });

    info!("Станция готова: адрес {:?}, спутник {:?}", gs_addr, sat_addr);
    info!("Команды: ping, fetch, help, quit");

    let mut buf = [0u8; 512];
    loop {
        while let Ok(cmd) = rx.try_recv() {
            match cmd.as_str() {
                "ping" => {
                    info!("→ ping на {:?}", sat_addr);
                    if let Err(e) = transport.send_datagram(sat_addr, b"ping").await {
                        error!("ping: {e:?}");
                    }
                }
                "fetch" => {
                    info!("→ fetch на {:?}", sat_addr);
                    if let Err(e) = transport.send_datagram(sat_addr, b"fetch").await {
                        error!("fetch: {e:?}");
                    }
                }
                "help" => info!("Команды: ping, fetch, help, quit"),
                "quit" | "exit" => return Ok(()),
                other => warn!("неизвестная команда: {other}"),
            }
        }

        match tokio::time::timeout(Duration::from_millis(250), transport.next(&mut buf)).await {
            Ok(Ok(TransportEvent::Beacon { payload })) => {
                match postcard::from_bytes::<Beacon>(payload) {
                    Ok(b) => info!(
                        "← BEACON sat={:?} interval={}s timestamp={}",
                        b.sat_addr, b.interval_sec, b.timestamp
                    ),
                    Err(_) => info!("← BEACON ({} байт)", payload.len()),
                }
            }
            Ok(Ok(TransportEvent::Datagram { from, data })) => {
                info!("← DATAGRAM от {:?}: {:?}", from, String::from_utf8_lossy(data));
            }
            Ok(Ok(TransportEvent::Session(mut s))) => {
                let peer = s.peer_addr();
                info!("← SESSION от {:?}", peer);
                let mut total = 0usize;
                let mut sbuf = [0u8; 8192];
                // Сессия забирает канал себе; станция возвращается к работе,
                // когда спутник закончит передачу или связь пропадёт.
                loop {
                    match tokio::time::timeout(Duration::from_secs(15), s.recv(&mut sbuf)).await {
                        Ok(Ok(data)) => {
                            total += data.len();
                            info!("  данные {} байт (всего {})", data.len(), total);
                        }
                        Ok(Err(e)) => {
                            warn!("  сессия закрыта: {e:?}");
                            break;
                        }
                        Err(_) => {
                            warn!("  сессия: 15 с без данных, завершаю");
                            break;
                        }
                    }
                }
                warn!(
                    "После сессии канал остаётся у неё (ограничение TransportImpl). Перезапустите программу."
                );
                return Ok(());
            }
            Ok(Err(e)) => error!("transport: {e:?}"),
            Err(_) => {} // таймаут — проверяем команды
        }
    }
}
