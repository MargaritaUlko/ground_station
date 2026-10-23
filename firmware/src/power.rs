//! Питание радиочипа на T-Beam.
//!
//! На T-Beam LoRa-чип питается не напрямую от 3,3 В, а от отдельной линии
//! контроллера питания (PMU). После сброса эта линия может быть выключена,
//! и тогда радиочип не отвечает по SPI. Здесь линия включается на 3,3 В.
//!
//! | Плата            | PMU     | Линия LoRa | Регистр напряжения | Регистр включения |
//! |------------------|---------|------------|--------------------|-------------------|
//! | T-Beam v1.2      | AXP2101 | ALDO2      | 0x93               | 0x90, бит 1       |
//! | T-Beam v1.0/v1.1 | AXP192  | LDO2       | 0x28, биты 7:4     | 0x12, бит 2       |
//!
//! Тип PMU определяется по регистру 0x03 (ID чипа).

use esp_hal::Blocking;
use esp_hal::i2c::master::I2c;

const PMU_ADDR: u8 = 0x34;
const REG_CHIP_ID: u8 = 0x03;

const AXP2101_ID: u8 = 0x4A;
const AXP2101_LDO_ONOFF_CTRL0: u8 = 0x90;
const AXP2101_ALDO2_VOLTAGE: u8 = 0x93;
const AXP2101_ALDO2_EN: u8 = 1 << 1;

const AXP192_ID: u8 = 0x03;
const AXP192_POWER_OUTPUT_CTRL: u8 = 0x12;
const AXP192_LDO23_VOLTAGE: u8 = 0x28;
const AXP192_LDO2_EN: u8 = 1 << 2;

pub enum Pmu {
    Axp2101,
    Axp192,
    Unknown,
}

impl Pmu {
    pub fn name(&self) -> &'static str {
        match self {
            Pmu::Axp2101 => "AXP2101",
            Pmu::Axp192 => "AXP192",
            Pmu::Unknown => "unknown",
        }
    }
}

fn read(i2c: &mut I2c<'_, Blocking>, reg: u8) -> Option<u8> {
    let mut v = [0u8; 1];
    i2c.write_read(PMU_ADDR, &[reg], &mut v).ok()?;
    Some(v[0])
}

fn write(i2c: &mut I2c<'_, Blocking>, reg: u8, value: u8) -> Option<()> {
    i2c.write(PMU_ADDR, &[reg, value]).ok()
}

pub fn enable_lora_power(i2c: &mut I2c<'_, Blocking>) -> Pmu {
    let pmu = match read(i2c, REG_CHIP_ID) {
        Some(AXP2101_ID) => Pmu::Axp2101,
        Some(AXP192_ID) => Pmu::Axp192,
        _ => return Pmu::Unknown,
    };

    match pmu {
        Pmu::Axp2101 => {
            // ALDO: 500..3500 мВ, шаг 100 мВ → (3300 - 500) / 100 = 28.
            let _ = write(i2c, AXP2101_ALDO2_VOLTAGE, 28);
            if let Some(ctrl) = read(i2c, AXP2101_LDO_ONOFF_CTRL0) {
                let _ = write(i2c, AXP2101_LDO_ONOFF_CTRL0, ctrl | AXP2101_ALDO2_EN);
            }
        }
        Pmu::Axp192 => {
            // LDO2: 1800..3300 мВ, шаг 100 мВ, биты 7:4 → (3300 - 1800) / 100 = 15.
            if let Some(v) = read(i2c, AXP192_LDO23_VOLTAGE) {
                let _ = write(i2c, AXP192_LDO23_VOLTAGE, (v & 0x0F) | (15 << 4));
            }
            if let Some(ctrl) = read(i2c, AXP192_POWER_OUTPUT_CTRL) {
                let _ = write(i2c, AXP192_POWER_OUTPUT_CTRL, ctrl | AXP192_LDO2_EN);
            }
        }
        Pmu::Unknown => {}
    }

    esp_hal::delay::Delay::new().delay_millis(20);
    pmu
}
