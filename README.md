# gs-tbeam — наземная станция lorasat на LILYGO T-Beam

Наземная станция для сети [asserthq/lorasat](https://github.com/asserthq/lorasat).
Состоит из двух программ:

```
┌──────────── ПК ────────────┐        ┌────────────── T-Beam ──────────────┐
│ host/  (gs-host)           │  USB   │ firmware/                          │
│ TransportImpl  (lorasat)   │◄──────►│ UART0 ◄──► ESP32 ◄──SPI──► LoRa ───┼──► эфир 868 МГц
│ LinkImpl       (lorasat)   │ COBS   │            Radio1262 / Radio       │
│ PhySerial      (этот репо) │        │            (sat-drivers, lorasat)  │
└────────────────────────────┘        └────────────────────────────────────┘
```

- **`firmware/`** — прошивка ESP32 на Rust (esp-hal + embassy). Плата работает прозрачным
  мостом: байты с USB передаёт в эфир, принятое из эфира отправляет на USB. Радиочип
  обслуживают драйверы из `sat-drivers` (`Radio1262` для SX1262, `Radio` для SX1276).
- **`host/`** — программа станции на ПК. Использует `LinkImpl` и `TransportImpl` из
  lorasat поверх `PhySerial` — реализации `PhyLayer` через COM-порт. Это тот же стек,
  что в `sandbox-pc/src/bin/gs`, только вместо UDP-заглушки `PhyRadioUDP` — настоящее радио.

Драйверы и протокол не копируются: Cargo скачивает их из репозитория lorasat при сборке.

## Структура

```
firmware/
  src/main.rs    инициализация, цикл «USB ↔ радио»
  src/radio.rs   определение чипа (SX1262/SX1276), создание драйвера из sat-drivers
  src/power.rs   включение питания LoRa через PMU (AXP2101 / AXP192)
  src/usb.rs     кадрирование на USB: [COBS(пакет)][0x00]
host/
  src/main.rs        режимы «станция» и «анализатор»
  src/phy_serial.rs  PhyLayer через COM-порт
patches/
  0001-drivers-feature-coils.patch   исправление для lorasat (см. шаг 1)
```

## Пины T-Beam

| Назначение | ESP32 |
|---|---|
| USB-UART (CH9102) TX / RX | GPIO1 / GPIO3 |
| PMU I2C SDA / SCL | GPIO21 / GPIO22 |
| LoRa SCK / MISO / MOSI / NSS | GPIO5 / GPIO19 / GPIO27 / GPIO18 |
| LoRa RESET | GPIO23 |
| SX1262: DIO1 / BUSY | GPIO33 / GPIO32 |
| SX1276: DIO0 | GPIO26 |

## Параметры радио

Задаются драйверами `sat-drivers` (`drivers/src/lora/radio.rs`), совпадают с `tx_flash` и `client`:

| Параметр | SX1262 (`Radio1262`) | SX1276 (`Radio`) |
|---|---|---|
| Частота | 868,000 МГц (`LORA_FREQ_HZ` в `firmware/src/main.rs`) | то же |
| SF / BW / CR | 12 / 125 кГц / 4/5 | **9** / 125 кГц / **4/8** |
| Преамбула, sync word, CRC | 8, приватный (0x12), включён | то же |

Прошивка сообщает тип чипа при загрузке (шаг 4). **Если на плате SX1276**, параметры
драйвера `Radio` не совпадают с `Radio1262`, и такая плата не услышит узлы на SX1262
(`tx_flash`, `client`). Это нужно исправлять в `sat-drivers`, а не здесь.

---

## Шаг 1. Исправление драйверов lorasat (один раз)

В текущем `master` lorasat крейт `sat-drivers` не собирается ни под ESP32, ни под STM32:
зависимость `drv8833-driver 0.1.0` (драйвер катушек, коммит «Доработка bdot, драйвер coils»)
использует `std::sync::Arc` и `Mutex`. Патч делает драйвер катушек опциональным
(фича `coils`).

1. На https://github.com/asserthq/lorasat нажмите **Fork** → форк появится как `MargaritaUlko/lorasat`.
2. В `cmd`:
   ```
   git clone https://github.com/asserthq/lorasat
   cd lorasat
   git checkout -b fix-drivers-no-std
   git am путь\к\gs-tbeam\patches\0001-drivers-feature-coils.patch
   git push -u origin fix-drivers-no-std
   ```
3. На GitHub в своём форке нажмите **Compare & pull request** → PR в `asserthq/lorasat`.

`firmware/Cargo.toml` содержит секцию `[patch]`, которая берёт драйверы из ветки
`fix-drivers-no-std` вашего форка. Когда PR примут, удалите эту секцию.
`host/` патч не нужен: на ПК `std` есть.

## Шаг 2. Инструменты (один раз, Windows)

1. Rust: https://rustup.rs → `rustup-init.exe`, установка по умолчанию.
2. Компилятор для ESP32 (архитектура Xtensa):
   ```
   cargo install espup --locked
   espup install
   ```
   После установки **закройте и откройте `cmd` заново**.
3. Утилита прошивки:
   ```
   cargo install espflash --locked
   ```

Проверка: `rustup toolchain list` показывает `esp`, `espflash --version` выводит версию.

## Шаг 3. Прошивка платы

1. Закройте всё, что держит COM-порт: веб-клиент Meshtastic, Arduino IDE, PlatformIO Monitor.
2. Подключите T-Beam по USB.
3. ```
   cd gs-tbeam\firmware
   cargo run --release
   ```
   `cargo run` собирает прошивку и запускает `espflash flash --monitor`: прошивает плату и
   открывает монитор порта. Первая сборка занимает несколько минут.
   Если `espflash` спросит порт, выберите порт платы.
   Если прошивка не стартует (`Connecting...` без прогресса), зажмите кнопку **BOOT**
   до начала записи.

Прошивка заменяет Meshtastic. Вернуть Meshtastic можно через flasher.meshtastic.org.

## Шаг 4. Проверка загрузки

В мониторе после прошивки должна появиться строка:

```
gs-bridge: pmu=AXP2101 radio=SX1262 ready
```

- `radio=` — какой чип стоит на плате (см. «Параметры радио»).
- `radio init failed: ...` — чип не ответил по SPI. Проверьте, что `pmu=` не `unknown`
  (иначе питание LoRa не включилось), и что антенна подключена.

Дальше в мониторе будут только двоичные данные. Выход из монитора: **Ctrl+C**.
Монитор держит COM-порт, поэтому закройте его перед запуском программы на ПК.

## Шаг 5. Программа на ПК

```
cd gs-tbeam\host
cargo run -- COM5            режим станции
cargo run -- COM5 --sniff    режим анализатора
```

Номер порта: **Диспетчер устройств → Порты (COM и LPT)**.

**Режим станции.** Стек lorasat: `LinkImpl` (адрес станции, по умолчанию 2) +
`TransportImpl`. Печатает маяки, датаграммы и сессии. Команды в консоли:
`ping`, `fetch`, `help`, `quit`. Адреса задаются флагами: `--addr 2 --sat 1`.

**Режим анализатора.** Печатает каждый кадр в эфире без фильтра по адресу:
`BEACON` (с разобранным `Beacon`), `DATA src= dst=` (с разбором датаграммы транспорта
или `ClientData`), `RAW` для байтов, которые не являются кадром lorasat.

## Шаг 6. Проверка радиосвязи

**Две платы T-Beam** (обе прошиты `firmware/`, обе на одном компьютере):

1. Окно 1: `cargo run -- COM5 --sniff` (плата Б).
2. Окно 2: `cargo run -- COM6` (плата А), введите `ping`.
3. В окне 1 появится `DATA src=0x00000002 dst=0x00000001 ... transport datagram: "ping"`.
   Значит, пакет прошёл: ПК → USB → ESP32 → SPI → SX1262 → эфир → SX1262 → SPI → ESP32 → USB → ПК.

**С прототипом спутника на STM32** (`sandbox-mcu`, бинарник `tx_flash`, адрес 1):

1. `cargo run -- COM5 --sniff` — раз в 10 с: `BEACON sat=Address(1) interval=10s timestamp=777`.
   Если рядом работает `client`, видны его кадры `DATA ... ClientData`.
2. `cargo run -- COM5` — станция печатает `← BEACON ...`.

---

## Что проверено

- `firmware/` собирается для `xtensa-esp32-none-elf` с драйверами lorasat (с патчем),
  образ для ESP32 создаётся (`espflash save-image`, 178 КБ).
- `host/` собирается с драйверами прямо из `asserthq/lorasat`.
- `host/` в обоих режимах проверен со спутником `sandbox-pc/src/bin/sat` через пару
  виртуальных COM-портов: станция принимает маяки, по `fetch` получает сессию с 2048 байтами.
- COBS в прошивке и на ПК дают одинаковые байты (20 000 случайных пакетов).
- На железе не проверено: нужна плата.

## Известные ограничения lorasat (не этого репозитория)

- `TransportImpl` при открытии сессии отдаёт канал сессии и не возвращает его. После
  `fetch` станция и спутник `sandbox-pc` не могут продолжать работу (спутник падает с
  `Busy` в `subsystems/src/comm/comm.rs:32`). Станция в этом случае завершается с сообщением.
- Спутник (`subsystems/src/comm/comm.rs`) обрабатывает только `fetch`, на `ping` не отвечает.
- `tx_flash` и `client` работают на канальном уровне (`LinkImpl`) без `TransportImpl`.
  Их маяки станция понимает (формат маяка общий), команды `ping`/`fetch` — нет.
- Адреса узлов в lorasat не согласованы: `tx_flash` — спутник 1, `sandbox-pc` — станция 1,
  спутник 0x11. Поэтому у станции по умолчанию адрес 2 и спутник 1.
- В `sandbox-mcu` под текущий API собираются `tx_flash`, `uart`, `uart_echo`, `flash_read`,
  `flash_erase`, `mag1`; `lora_868_tx`, `lora_868_rx`, `gs`, `satellite`, `client` и
  `firmware` — нет (используют старые `sat_core::layer`, `SimpleTransport`, `LinkImpl::new(radio)`).

## Как выложить на GitHub

```
cd gs-tbeam
git init
git add .
git commit -m "Наземная станция lorasat на T-Beam"
git branch -M main
git remote add origin https://github.com/MargaritaUlko/gs-tbeam
git push -u origin main
```

Перед этим создайте на GitHub пустой репозиторий `gs-tbeam` (без README).
