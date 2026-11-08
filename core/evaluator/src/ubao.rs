//! Virtual board (`ubao`): pins and a clock that exist only in the host. A program drives pins
//! with `ubao_andika` and reads them with `ubao_soma`; `ubao_subiri` moves the board's clock on
//! and `ubao_saa` reads it. Nothing here touches hardware, and nothing depends on wall time, so a
//! run is reproducible: the same program with the same scenario gives the same events.
//!
//! A scenario (`anza`) is a list of scripted input levels: a read returns the level the scenario
//! gives for that pin at the current virtual time (the latest change at or before it, `false` if
//! there is none). Every pin write, pin read and clock step is recorded as a [`Tukio`], numbered
//! from 0 in the order it happened.
//!
//! The state is per thread, like the other host state a program's tasks share, and is reset by
//! [`anza`] at the start of a run.

use std::cell::RefCell;

/// A scripted input: pin `pini` reads `hali` from virtual time `saa` (milliseconds) on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ingizo {
    pub saa: i64,
    pub pini: i64,
    pub hali: bool,
}

/// One recorded event. `aina` is `"andika"` (pin write), `"soma"` (pin read) or `"subiri"`
/// (clock step, `pini` is `-1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tukio {
    pub hatua: u64,
    pub aina: &'static str,
    pub pini: i64,
    pub hali: bool,
    pub saa: i64,
}

#[derive(Default)]
struct Bodi {
    saa: i64,
    ingizo: Vec<Ingizo>,
    tukio: Vec<Tukio>,
}

thread_local! {
    static BODI: RefCell<Bodi> = RefCell::new(Bodi::default());
}

/// Start a run: reset the clock and the events, and install the scenario's inputs.
pub fn anza(ingizo: Vec<Ingizo>) {
    BODI.with(|b| {
        *b.borrow_mut() = Bodi {
            ingizo,
            ..Bodi::default()
        }
    });
}

/// The events recorded since the last [`anza`], in order. Taking them leaves the list empty.
pub fn tukio() -> Vec<Tukio> {
    BODI.with(|b| std::mem::take(&mut b.borrow_mut().tukio))
}

/// The board's clock, in virtual milliseconds.
pub fn saa() -> i64 {
    BODI.with(|b| b.borrow().saa)
}

impl Tukio {
    /// The event as one JSON object, the form the page receives (see `platform::write_event`).
    pub fn to_json(&self) -> String {
        format!(
            "{{\"hatua\":{},\"aina\":\"{}\",\"pini\":{},\"hali\":{},\"saa\":{}}}",
            self.hatua, self.aina, self.pini, self.hali, self.saa
        )
    }
}

fn rekodi(b: &mut Bodi, aina: &'static str, pini: i64, hali: bool) {
    let hatua = b.tukio.len() as u64;
    let t = Tukio {
        hatua,
        aina,
        pini,
        hali,
        saa: b.saa,
    };
    crate::platform::write_event(&t.to_json());
    b.tukio.push(t);
}

/// Drive pin `pini` to `hali`.
pub fn andika(pini: i64, hali: bool) {
    BODI.with(|b| rekodi(&mut b.borrow_mut(), "andika", pini, hali));
}

/// The level of pin `pini` now, from the scenario (`false` when the scenario says nothing).
pub fn soma(pini: i64) -> bool {
    BODI.with(|b| {
        let mut b = b.borrow_mut();
        let now = b.saa;
        let hali = b
            .ingizo
            .iter()
            .filter(|i| i.pini == pini && i.saa <= now)
            .max_by_key(|i| i.saa)
            .map(|i| i.hali)
            .unwrap_or(false);
        rekodi(&mut b, "soma", pini, hali);
        hali
    })
}

/// Advance the clock by `ms` milliseconds (negative steps are ignored).
pub fn subiri(ms: i64) {
    BODI.with(|b| {
        let mut b = b.borrow_mut();
        let ms = ms.max(0);
        b.saa = b.saa.saturating_add(ms);
        rekodi(&mut b, "subiri", -1, false);
    });
}
