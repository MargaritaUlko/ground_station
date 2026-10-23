//! Заглушки для `defmt` на ПК.
//!
//! Драйверы lorasat (`sat-drivers`, `lora-phy`, `embedded-hal`) пишут логи через `defmt` —
//! систему логов для микроконтроллеров. Ей нужны функции и символы, которые на
//! микроконтроллере дают `defmt-rtt` и скрипт линковщика `defmt.x`. На ПК их нет, и
//! линковщик Windows (`link.exe`) завершается ошибкой `LNK2001: _defmt_write` и т. п.
//! Здесь они определены пустыми: логи `defmt` на ПК отбрасываются.

/// Логгер `defmt`, который ничего не выводит (`_defmt_acquire/_release/_flush/_write`).
#[defmt::global_logger]
struct NullLogger;

unsafe impl defmt::Logger for NullLogger {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

/// `defmt::panic!` на ПК — обычная паника Rust (`_defmt_panic`).
#[defmt::panic_handler]
fn defmt_panic() -> ! {
    panic!("defmt panic")
}

// Метка времени в логах `defmt` (`_defmt_timestamp`).
defmt::timestamp!("");

// Границы уровней логов. На микроконтроллере их задаёт скрипт `defmt.x`.
#[unsafe(no_mangle)]
static __DEFMT_MARKER_TRACE_START: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_TRACE_END: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_DEBUG_START: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_DEBUG_END: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_INFO_START: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_INFO_END: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_WARN_START: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_WARN_END: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_ERROR_START: u8 = 0;
#[unsafe(no_mangle)]
static __DEFMT_MARKER_ERROR_END: u8 = 0;
