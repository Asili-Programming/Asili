# blink-esp32

An LED blinking on an ESP32, written in Asili only — no C. `kuu` is `#[wazi]`, so
`pata jenga --lengo esp32` builds it as the firmware's entry point (`app_main`, which ESP-IDF calls
at start-up), and `ubao_andika`/`ubao_subiri` compile to the chip's GPIO registers and a wait on
its cycle counter. GPIO 2 is the LED on most ESP32 development boards; change `LED` for another.

```
$ pata jenga --lengo esp32
msimbo asilia: ./kilele/blink.nguvu
kifaa (ESP32): ./kilele/blink-esp32.o na ./kilele/blink.h — kazi: kuu (app_main)
kifaa (ESP32): kazi 'kuu' — mrundikano ≤ baiti 208
imejengwa: ./kilele/blink.asb
```

## On the board

`firmware/` is an ESP-IDF project with no source of its own: its `main` component links
`kilele/blink-esp32.o`. With ESP-IDF set up (`. $IDF_PATH/export.sh`):

```
$ cd firmware
$ idf.py set-target esp32
$ idf.py -p /dev/ttyUSB0 build flash
```

The LED goes on and off every half second. `sdkconfig.defaults` turns off the task watchdog,
since `kuu` never returns. ESP-IDF's own start-up (bootloader, FreeRTOS) still runs before `kuu`.

## On the build machine

The same `kuu` drives the virtual board (`docs/spec/05-standard-library.md`, `ubao`), where
`ubao_subiri` moves the board's clock without waiting — so `pata tenda` runs the endless loop
flat out; stop it with Ctrl-C.

See `docs/howto/08-build-for-esp32.md`.
