/* asili_kifaa_esp32.c: the device runtime for `pata jenga --lengo esp32`, linked beside the
 * object it writes (and the header's functions). It mirrors `driver/kifaa/src/lib.rs`
 * (asili-kifaa) function for function: stock Rust cannot build for Xtensa, so the ESP32 gets
 * this C copy. Keep the two in step — each computes what the host's runtime
 * (`core/evaluator/src/native.rs`) computes.
 *
 * Strict code calls these by name for what it does not do inline: reading the caller's lists,
 * `%` and `**` on numbers, and the conversions of the bitwise operators. No allocation, no
 * threads, no files. `pow` comes from the C library, whose results can differ from the
 * host's in the last bit (as on Cortex-M). */
#include <math.h>
#include <stdint.h>

/* A list as the caller passes it (`asili_orodha` in the generated header). */
typedef struct {
    double *data;
    uint32_t len;
} asili_kifaa_orodha;

/* `Kind::F64.code()`: the only representation a device list has. */
#define ASILI_F64_CODE 7u
/* Status of a call that cannot be made on a device (`native::STATUS_FAIL << 32`). */
#define ASILI_FAIL ((uint64_t)2 << 32)

/* Data pointer of list register `reg` of `frame`, or null when the code wants the list held
 * another way. */
uint64_t asili_kifaa_list_ptr(const asili_kifaa_orodha *frame, uint32_t reg, uint32_t want) {
    if (want != ASILI_F64_CODE) {
        return 0;
    }
    return (uint64_t)(uintptr_t)frame[reg].data;
}

/* Length of list register `reg` of `frame`. */
int64_t asili_kifaa_list_len(const asili_kifaa_orodha *frame, uint32_t reg) {
    return (int64_t)frame[reg].len;
}

double asili_kifaa_fmod(double a, double b) { return fmod(a, b); }

double asili_kifaa_pow(double a, double b) { return pow(a, b); }

/* Saturating `f64 as i64` (NaN -> 0), as Rust converts. */
int64_t asili_kifaa_float_to_int_sat(double a) {
    if (a != a) {
        return 0;
    }
    if (a >= 9223372036854775808.0) {
        return INT64_MAX;
    }
    if (a <= -9223372036854775808.0) {
        return INT64_MIN;
    }
    return (int64_t)a;
}

/* The language's shift amount: `f64 as i32` (saturating, NaN -> 0), anything outside 0..=63
 * -> 0. */
int64_t asili_kifaa_shift_amount(double a) {
    int32_t shift;
    if (a != a) {
        shift = 0;
    } else if (a >= 2147483647.0) {
        shift = INT32_MAX;
    } else if (a <= -2147483648.0) {
        shift = INT32_MIN;
    } else {
        shift = (int32_t)a;
    }
    return (shift >= 0 && shift <= 63) ? shift : 0;
}

/* An instruction native code hands to the host. On a device that only happens on an error path
 * (an index out of range): the call fails (`native::STATUS_FAIL`). */
uint64_t asili_kifaa_exec(void *host, void *frame, uint32_t function, uint32_t pc) {
    (void)host;
    (void)frame;
    (void)function;
    (void)pc;
    return 2;
}

/* A direct call too deep for the stack: a device has no host call path, so the call fails. */
uint64_t asili_kifaa_call_host(void *host, uint32_t function, double *nums) {
    (void)host;
    (void)function;
    (void)nums;
    return ASILI_FAIL;
}

/* The call depth reached its limit. */
uint64_t asili_kifaa_depth_error(void *host) {
    (void)host;
    return ASILI_FAIL;
}
