# Blink on an ESP32, in Asili only: `kuu` is `#[wazi]`, so `pata jenga --lengo esp32` builds it
# as the firmware's entry point (`app_main`), and `ubao_andika` drives GPIO 2 — the LED on most
# ESP32 development boards. On the build machine the same program drives the virtual board.

leta ubao

thabiti LED: Namba = 2

#[wazi]
kazi kuu(hoja: Orodha<Neno>) -> Tupu {
    wakati kweli {
        ubao_andika(LED, kweli)
        ubao_subiri(500)
        ubao_andika(LED, si_kweli)
        ubao_subiri(500)
    }
}
