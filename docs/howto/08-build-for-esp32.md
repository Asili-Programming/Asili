# Build strict code for ESP32

`pata jenga --lengo esp32` compiles a program's strict functions (`#[salama]`, see
`docs/language/07-mfumo-wa-aina.md`, "Strict code") to Xtensa LX6 machine code for the ESP32,
using Asili's own backend (`nguvu`) — no external compiler is involved in producing the code. The
result is an object file and a C header that an ESP-IDF firmware links like any other library.
The exports have the same C interface as the Cortex-M target (`docs/howto/07-build-for-cortex-m.md`).

It also builds `#[wazi]` functions, which drive the chip's pins and wait in real time through
`ubao` ("Pins, the clock, and firmware in Asili only" below), and a `#[wazi] kuu` as the
firmware's entry point, so a firmware can be written with no C at all.

For the smallest complete examples see `examples/habari-esp32` (an ESP-IDF project that prints
what an Asili function computes) and `examples/blink-esp32` (an LED blinking, in Asili only).

## Build

```
$ cd examples/kidhibiti
$ pata jenga --lengo esp32
salama: kazi 'dhibiti' — hatua ≤ 69, kumbukumbu ≤ baiti 200
salama: kazi 'wastani' — hatua ≤ 49, kumbukumbu ≤ baiti 104
msimbo asilia: ./kilele/kidhibiti.nguvu
kifaa (ESP32): ./kilele/kidhibiti-esp32.o na ./kilele/kidhibiti.h — kazi: wastani, dhibiti
kifaa (ESP32): kazi 'wastani' — mrundikano ≤ baiti 48
kifaa (ESP32): kazi 'dhibiti' — mrundikano ≤ baiti 64
imejengwa: ./kilele/kidhibiti.asb
```

Each `mrundikano` line is the most stack a call of that export takes in Asili's code: its frame
plus the deepest chain of frames through the functions it calls (the compiler runtime's and the
C library's own frames, small, not included). Give the calling task at least that much more
than it otherwise needs.

`[jenga] lengo = "esp32"` in `pata.toml` does the same without the flag, and
`#[sharti(lengo = "esp32")]` selects items for this target as for any other.

- `kilele/<jina>-esp32.o` — an ELF32 relocatable object (Xtensa) with one global function per
  strict function, `asili_<kazi>`, using the windowed ABI that ESP-IDF code uses. An export is
  the function itself, reading its arguments from `hoja` and writing its number to `matokeo`
  (a function that cannot be compiled that way is called through a small wrapper instead). The
  code uses the ESP32's 16-bit code-density instructions wherever one fits (about a quarter
  smaller).
- `kilele/<jina>.h` — their C declarations.

## Link into an ESP-IDF firmware

Copy the object and the header into a component and archive the object with it — nothing else:
the ESP32 needs no runtime library of Asili's own.

```cmake
idf_component_register(SRCS "main.c" INCLUDE_DIRS ".")
target_sources(${COMPONENT_LIB} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/kidhibiti-esp32.o")
```

What the Cortex-M target calls in its device runtime (`driver/kifaa`) — reading the caller's
lists, the conversions behind the bitwise operators, floor and ceiling, the failing status of
host-only calls — is compiled into the ESP32 code itself (exact bit arithmetic for the
conversions). The code calls only what every ESP-IDF build links: the GCC runtime's 64-bit and
double-precision routines (`__adddf3`, `__muldi3`, ...) and the C library's `fmod` (`%`, exact in
any correct library) and `pow` (`**`). Every call goes through a literal holding the callee's
address (`l32r` + `callx8`), so code placed in flash reaches functions in ROM or IRAM.

## Call from C

```c
#include "kidhibiti.h"

static double sampuli[8];

void control_tick(double lengo, double *marekebisho) {
    asili_dhibiti_hoja hoja = { { sampuli, 8 }, lengo };
    if (asili_dhibiti(&hoja, marekebisho) != 0) {
        enter_safe_state();   /* the call failed: the output was not written */
    }
}
```

The calling convention is the Cortex-M one: `int32_t asili_<kazi>(const asili_<kazi>_hoja *hoja,
double *matokeo)`, arguments eight bytes each in `hoja`, 0 returned on success and non-zero when
the call failed (`matokeo` then not written), lists passed by reference.

## What runs on a device

The same subset as Cortex-M: exported functions take only `Namba`, `Ukweli` and `Orodha<Namba>`
parameters, there is no allocation, and anything else fails the build naming the function and the
reason (`kazi 'f' haiwezi kujengwa kwa ESP32: ...`). The exports are the `#[salama]` functions and
the `#[wazi]` ones (other than `kuu`). Functions call each other directly, including ones
returning `Tupu`.

## Pins, the clock, and firmware in Asili only

`ubao` (`docs/spec/05-standard-library.md`) is the same interface on every target: the virtual
board on the build machine and in the browser, the chip's hardware here. On the ESP32:

| Call | Compiles to |
|---|---|
| `ubao_andika(n, hali)` | GPIO `n` put in GPIO function (IO_MUX), routed from `GPIO_OUT` and enabled as an output, then its bit written to the set or clear register |
| `ubao_soma(n)` | GPIO `n` put in GPIO function with its input enabled, then its bit of `GPIO_IN` |
| `ubao_subiri(ms)` | a wait on the cycle counter, one millisecond at a time, at the clock the chip runs (read from its clock registers: 80/160/240 MHz on the PLL, or the 40 MHz crystal's) |
| `ubao_saa()` | timer group 1's timer 0, counting microseconds off the APB clock, read in whole milliseconds; the timer is started at 0 by the entry point of a program that reads it (or, without one, by the first call) |

Every register access is preceded by `memw`, as GCC orders volatile accesses on this core. Pin
`n` is GPIO `n` and must be a constant the build can see (a literal or a `thabiti`); a pin the
chip does not have, one of the flash's (6-11), or an input-only pin (34-39) written is a build
error.

`ubao_saa` is real time on the device, where the build machine's virtual clock moves only with
`ubao_subiri`, so a program that times its own computation reads different values on the two.
It uses timer group 1's timer 0, which the firmware must leave to it.

A `#[wazi] kuu` (its `hoja` unused) is built as `void app_main(void)`, which ESP-IDF calls at
start-up, so the firmware needs no C source: its `main` component only links the object.

```
# src/kuu.as
leta ubao

#[wazi]
kazi kuu(hoja: Orodha<Neno>) -> Tupu {
    wakati kweli {
        ubao_andika(2, kweli)
        ubao_subiri(500)
        ubao_andika(2, si_kweli)
        ubao_subiri(500)
    }
}
```

```cmake
# firmware/main/CMakeLists.txt
idf_component_register()
target_link_libraries(${COMPONENT_LIB} INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/../../kilele/blink-esp32.o")
```

A `kuu` that never returns keeps ESP-IDF's idle task from running, so set
`CONFIG_ESP_TASK_WDT_INIT=n` (`sdkconfig.defaults`). On a real ESP32, a pin written reads back
the level written, `ubao_subiri(1000)` takes 1000.3 ms, and `ubao_saa` measures a 250 ms wait
as 250 ms. Cortex-M has no `ubao` yet: a function
using it fails that build.

## Results are the same as on the build machine

The ESP32 has no double-precision unit: doubles are computed by the GCC runtime's IEEE-exact
routines, and 64-bit integers in register pairs, so every operator gives what native code on the
build machine gives. `core/evaluator/tests/kifaa.rs` builds the `kidhibiti` example, hand-written
cases and random strict programs for the ESP32, links them into an ESP-IDF firmware, runs it under
Espressif's `qemu-system-xtensa`, and requires every result — or failure — to equal native code's
bit for bit. It runs when `ASILI_ESP_IDF` points to an ESP-IDF checkout with its tools (including
`qemu-xtensa`) installed:

```
$ ASILI_ESP_IDF=~/esp/esp-idf cargo test -p asili-evaluator --test kifaa
```

As on Cortex-M, `**` comes from the C library and can differ from the build machine's in the last
bit.

## Not yet

- **Faster doubles than C.** Double arithmetic is the GCC runtime's software routines (about
  0.37 µs per addition on an ESP32 at ESP-IDF's default 160 MHz), the same ones C calls, so float-heavy code runs at
  C's speed and no faster: the controller example matches the same functions in C (`gcc -O2`)
  — `wastani` 4.25 µs against 4.19, `dhibiti` 5.32 against 5.20. Values live in the three
  register pairs a call preserves (`a2`–`a7`) or in stack slots shared by values never live
  together; constants are rebuilt where read; a list element feeding a double operation is
  loaded straight into its argument registers; an unrolled loop's index checks are one check
  of the largest index (the call fails the same way whichever index is out of range).
- **No ESP-IDF at all.** ESP-IDF's bootloader and start-up (and FreeRTOS) still run before
  `kuu`; a complete flash image of Asili's own is planned. Peripherals beyond GPIO and the clock
  (serial output, timers, buses) are the firmware's.
