# habari-esp32

Hello world on an ESP32. `fibonacci` is strict code (`#[salama]`), so `pata jenga --lengo esp32`
compiles it to Xtensa machine code; a few lines of C firmware print the greeting and the
numbers it computes. (Asili code on a device takes and returns numbers only, so the printing is
the firmware's.) `kuu` prints the same on the build machine:

```
$ pata jenga --lengo esp32
salama: kazi 'fibonacci' — hatua ≤ 919, kumbukumbu ≤ baiti 72
msimbo asilia: ./kilele/habari.nguvu
kifaa (ESP32): ./kilele/habari-esp32.o na ./kilele/habari.h — kazi: fibonacci
kifaa (ESP32): kazi 'fibonacci' — mrundikano ≤ baiti 64
imejengwa: ./kilele/habari.asb
$ pata tenda kilele/habari.asb
Habari, dunia!
fibonacci(0) = 0
...
fibonacci(10) = 55
```

## On the board

`firmware/` is an ESP-IDF project that links `kilele/habari-esp32.o` and includes
`kilele/habari.h` straight from the build output. With ESP-IDF set up (`. $IDF_PATH/export.sh`):

```
$ cd firmware
$ idf.py set-target esp32
$ idf.py -p /dev/ttyUSB0 build flash monitor
Habari, dunia! (kutoka ESP32)
fibonacci(0) = 0
fibonacci(1) = 1
...
fibonacci(10) = 55
```

The loop in `fibonacci` runs a constant 70 times with the work inside an `ikiwa`: strict code
needs loops whose bound is known at build time. See `docs/howto/08-build-for-esp32.md`.
