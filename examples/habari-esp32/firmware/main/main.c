/* Habari, dunia on an ESP32: the firmware prints, Asili's `fibonacci` (compiled by
   `pata jenga --lengo esp32`) computes. */
#include <stdio.h>
#include "habari.h"

void app_main(void) {
    printf("Habari, dunia! (kutoka ESP32)\n");
    for (int n = 0; n <= 10; n++) {
        asili_fibonacci_hoja hoja = { n };
        double f;
        if (asili_fibonacci(&hoja, &f) == 0) {
            printf("fibonacci(%d) = %.0f\n", n, f);
        } else {
            printf("fibonacci(%d) imeshindwa\n", n); /* the call failed: f was not written */
        }
    }
}
