//! Export thunks: the C interface `int32_t asili_f(const hoja *h, double *matokeo)` around a
//! function's ordinary or direct entry, and `void app_main(void)` around a `#[wazi] kuu`.

use super::*;

/// What an export thunk needs to know about its function (see [`thunk`]).
pub struct ThunkSpec {
    /// Numeric registers and list registers of the function's frame.
    pub num_regs: u32,
    pub list_regs: u32,
    /// Registers that start with a constant: (register, bits of the double).
    pub num_consts: Vec<(u32, u64)>,
    /// Parameters in order: (register, whether it is a list).
    pub params: Vec<(u32, bool)>,
    /// `Return`s of a number or truth value: (pc, register returned).
    pub returns: Vec<(u32, u32)>,
    /// Call the function's direct entry (a number out; list references after the numeric
    /// registers, see `lower::Ctx::direct_lists`): `(host, nums)` in, the number back in
    /// registers with status 0, so no `Return` needs finding.
    pub direct: bool,
    /// The firmware's entry point, `void app_main(void)` (the program's `kuu`): no arguments
    /// read, nothing returned (`params` and `returns` are empty).
    pub entry_point: bool,
    /// For the entry point: restart the board's clock first (the helper of this index in
    /// [`HELPERS`], `RtFn::BoardClock`, called with 1), so a program's clock starts with it.
    pub start_clock: Option<u32>,
}

/// Status kind in the high word of a function's return value meaning "returned"
/// (`native::STATUS_RETURN`).
pub(super) const STATUS_RETURN: i32 = 3;

/// The export `int32_t asili_f(const hoja *h, double *matokeo)`: builds the function's frame
/// on the stack from `hoja` (a2), calls its ordinary entry (the one entry in `calls`, to be
/// linked), and stores the number returned at `matokeo` (a3). Returns 0, or the non-zero
/// status kind when the call failed.
pub fn thunk(spec: &ThunkSpec) -> Result<Code, String> {
    // Scratch of the thunk's own (it allocates nothing): a4..a7.
    const TS0: Pair = (4, 5);
    const TS1: Pair = (6, 7);
    let nums = 8 * spec.num_regs;
    let lists = 8 * spec.list_regs;
    let host = nums + lists; // depth, runtime table, stack limit: 24 bytes, all zero
    let size = (host + 24).div_ceil(8) * 8;
    let frame = size.div_ceil(16) * 16 + 32;
    if frame > 32760 {
        return Err("kazi ina rejista nyingi mno kwa ESP32".into());
    }
    let (hoja, matokeo) = (2u8, 3u8);
    let mut a = Asm::new();
    let mut pool = Pool::default();
    a.emit(xt32::entry(frame))?;
    // Zero the frame: registers, list references, host fields. A direct entry reads only its
    // parameters and the host fields (its constants and other registers start in code, and
    // its number comes back in registers), so it needs only the host fields zeroed.
    a.emit(xt32::movi(TS1.0, 0))?;
    let zero_from = if spec.direct { host } else { 0 };
    for off in (zero_from..size).step_by(4) {
        store_word(&mut a, &mut pool, TS1.0, off)?;
    }
    // Constants the registers start with (read back by the ordinary entry's `Return`s).
    let consts: &[(u32, u64)] = if spec.direct { &[] } else { &spec.num_consts };
    for &(reg, bits) in consts {
        pool.const32(&mut a, TS0.0, bits as u32)?;
        pool.const32(&mut a, TS0.1, (bits >> 32) as u32)?;
        store_word(&mut a, &mut pool, TS0.0, 8 * reg)?;
        store_word(&mut a, &mut pool, TS0.1, 8 * reg + 4)?;
    }
    // Arguments, eight bytes each in `hoja`.
    for (i, &(reg, is_list)) in spec.params.iter().enumerate() {
        let from = 8 * i as u32;
        if from + 4 > 1020 {
            return Err("ina hoja nyingi mno".into());
        }
        a.emit(xt32::l32i(TS0.0, hoja, from))?;
        a.emit(xt32::l32i(TS0.1, hoja, from + 4))?;
        let to = if is_list { nums + 8 * reg } else { 8 * reg };
        store_word(&mut a, &mut pool, TS0.0, to)?;
        store_word(&mut a, &mut pool, TS0.1, to + 4)?;
    }
    if spec.direct {
        // direct entry(host, nums): status in a10:a11 (0: returned), the number in a12:a13.
        pool.const32(&mut a, T, host)?;
        a.word(xt32::add(ARG0, SP, T));
        a.word(xt32::mov(ARG0 + 1, SP));
        pool.call_direct(&mut a, 0);
        let failed = a.new_label();
        a.word(xt32::or(T, ARG0, ARG0 + 1));
        a.emit(xt32::movi(T2, 0))?;
        a.branch(BranchCond::Ne, T, T2, failed);
        a.emit(xt32::s32i(ARG0 + 2, matokeo, 0))?;
        a.emit(xt32::s32i(ARG0 + 3, matokeo, 4))?;
        a.emit(xt32::movi(2, 0))?;
        a.word(xt32::RETW);
        a.bind(failed);
        a.word(xt32::mov(2, ARG0 + 1)); // the status kind: non-zero for every failure
        a.word(xt32::RETW);
        let mut code = pool.finish(a)?;
        code.frame = frame;
        return Ok(code);
    }
    if let Some(h) = spec.start_clock {
        a.emit(xt32::movi(ARG0, 1))?;
        a.emit(xt32::movi(ARG0 + 1, 0))?;
        pool.l32r(&mut a, T, Lit::Helper(h));
        a.word(xt32::callx(8, T).expect("callx8"));
    }
    // entry(runtime: unused on a device, host, frame = list references, nums)
    a.emit(xt32::movi(ARG0, 0))?;
    pool.const32(&mut a, T, host)?;
    a.word(xt32::add(ARG0 + 1, SP, T));
    pool.const32(&mut a, T, nums)?;
    a.word(xt32::add(ARG0 + 2, SP, T));
    a.word(xt32::mov(ARG0 + 3, SP));
    pool.call_direct(&mut a, 0);
    if spec.entry_point {
        a.word(xt32::RETW);
        let mut code = pool.finish(a)?;
        code.frame = frame;
        return Ok(code);
    }
    // a11 = status kind, a10 = the pc of the `Return` reached.
    let fail = a.new_label();
    let done = a.new_label();
    let ok = a.new_label();
    a.emit(xt32::movi(T2, STATUS_RETURN))?;
    a.branch(BranchCond::Eq, ARG0 + 1, T2, ok);
    a.jump(fail);
    a.bind(ok);
    for &(pc, reg) in &spec.returns {
        let next = a.new_label();
        pool.const32(&mut a, T2, pc)?;
        a.branch(BranchCond::Ne, ARG0, T2, next);
        let off = 8 * reg;
        let (b, o) = if off + 4 <= 1020 {
            (SP, off)
        } else {
            pool.const32(&mut a, T, off)?;
            a.word(xt32::add(T, SP, T));
            (T, 0)
        };
        a.emit(xt32::l32i(TS0.0, b, o))?;
        a.emit(xt32::l32i(TS0.1, b, o + 4))?;
        a.emit(xt32::s32i(TS0.0, matokeo, 0))?;
        a.emit(xt32::s32i(TS0.1, matokeo, 4))?;
        a.jump(done);
        a.bind(next);
    }
    a.bind(done);
    a.emit(xt32::movi(2, 0))?;
    a.word(xt32::RETW);
    a.bind(fail);
    a.word(xt32::mov(2, ARG0 + 1));
    a.word(xt32::RETW);
    let mut code = pool.finish(a)?;
    code.frame = frame;
    Ok(code)
}

/// `[sp + off] = r` for any frame offset.
pub(super) fn store_word(a: &mut Asm, pool: &mut Pool, r: u8, off: u32) -> Result<(), String> {
    if off <= 1020 {
        a.emit(xt32::s32i(r, SP, off))
    } else {
        pool.const32(a, T, off)?;
        a.word(xt32::add(T, SP, T));
        a.emit(xt32::s32i(r, T, 0))
    }
}
