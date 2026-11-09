# Habari, dunia — on the build machine and on an ESP32.
#
# `fibonacci` is strict (`#[salama]`), so `pata jenga --lengo esp32` compiles it to Xtensa
# machine code the ESP32 runs; `kuu` runs on the build machine only.

#[salama]
kazi fibonacci(n: Namba) -> Namba {
    weka a = 0
    weka b = 1
    kwa i kutoka 0 hadi 70 {
        ikiwa i < n {
            weka c = a + b
            a = b
            b = c
        }
    }
    rejesha a
}

kazi kuu(hoja: Orodha<Neno>) -> Tupu {
    chapisha("Habari, dunia!")
    kwa n kutoka 0 hadi 11 {
        chapisha("fibonacci(" + (n kama Neno) + ") = " + (fibonacci(n) kama Neno))
    }
}
