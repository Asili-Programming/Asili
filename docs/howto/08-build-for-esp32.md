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
reason (`kazi salama 'f' haiwezi kujengwa kwa ESP32: ...`).

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

- **Speed and size.** Every IR value lives in a stack slot (no register allocation yet); values
  that are never live at the same time share a slot, which keeps frames small (the
  `kidhibiti`-style sorting function `panga` needs 208 bytes), but each instruction still loads
  and stores its operands.
- Start-up code, RTOS integration and peripherals are the firmware's: the object is a library.
