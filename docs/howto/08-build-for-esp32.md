# Build strict code for ESP32

`pata jenga --lengo esp32` compiles a program's strict functions (`#[salama]`, see
`docs/language/07-mfumo-wa-aina.md`, "Strict code") to Xtensa LX6 machine code for the ESP32,
using Asili's own backend (`nguvu`) — no external compiler is involved in producing the code. The
result is an object file and a C header that an ESP-IDF firmware links like any other library.
The exports have the same C interface as the Cortex-M target (`docs/howto/07-build-for-cortex-m.md`).

## Build

```
$ cd examples/kidhibiti
$ pata jenga --lengo esp32
salama: kazi 'dhibiti' — hatua ≤ 69, kumbukumbu ≤ baiti 200
salama: kazi 'wastani' — hatua ≤ 49, kumbukumbu ≤ baiti 104
msimbo asilia: ./kilele/kidhibiti.nguvu
kifaa (ESP32): ./kilele/kidhibiti-esp32.o na ./kilele/kidhibiti.h — kazi: wastani, dhibiti
imejengwa: ./kilele/kidhibiti.asb
```

`[jenga] lengo = "esp32"` in `pata.toml` does the same without the flag, and
`#[sharti(lengo = "esp32")]` selects items for this target as for any other.

- `kilele/<jina>-esp32.o` — an ELF32 relocatable object (Xtensa) with one global function per
  strict function, `asili_<kazi>`, using the windowed ABI that ESP-IDF code uses.
- `kilele/<jina>.h` — their C declarations.

## Link into an ESP-IDF firmware

Copy the object, the header and the device runtime `driver/esp32/asili_kifaa_esp32.c` into a
component, and archive the object with it so the runtime functions it calls resolve:

```cmake
idf_component_register(SRCS "main.c" "asili_kifaa_esp32.c" INCLUDE_DIRS ".")
target_sources(${COMPONENT_LIB} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/kidhibiti-esp32.o")
```

`asili_kifaa_esp32.c` is the C mirror of the Rust device runtime `driver/kifaa` (stock Rust cannot
build for Xtensa): the `asili_kifaa_*` functions the code calls by name for list access, `%`,
`**` and the conversions behind the bitwise operators. The code also calls the GCC runtime's
64-bit and double-precision routines (`__adddf3`, `__muldi3`, ...), which every ESP-IDF build
links. Every call goes through a literal holding the callee's address (`l32r` + `callx8`), so
code placed in flash reaches functions in ROM or IRAM.

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
reason (`kazi salama 'f' haiwezi kujengwa kwa ESP32: ...`). Lists of narrow integers are not
supported on ESP32 yet.

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

- **Speed and size.** Every IR value lives in its own stack slot (no register allocation yet), so
  frames and code are large: a strict function with long unrolled loops can need several
  kilobytes of stack, more than ESP-IDF's default main task stack (3.5 KB). Give the calling task
  enough stack (`CONFIG_ESP_MAIN_TASK_STACK_SIZE`).
- Lists of narrow integers (`U8`..`I32` element kinds).
- Start-up code, RTOS integration and peripherals are the firmware's: the object is a library.
