//! The ESP32's runtime, compiled from IR into the object itself: list access, the host-only
//! calls' failing status, the float conversions (helpers), and the board's GPIO pins and clock
//! (registers from ESP-IDF's `soc/esp32` headers).

use super::*;

/// `GPIO_OUT_W1TS_REG` and the other GPIO registers (`DR_REG_GPIO_BASE` 0x3FF44000); the `1`
/// registers serve pins 32 and up.
pub(super) const GPIO_OUT_W1TS: u32 = 0x3FF4_4008;

pub(super) const GPIO_OUT_W1TC: u32 = 0x3FF4_400C;

pub(super) const GPIO_OUT1_W1TS: u32 = 0x3FF4_4014;

pub(super) const GPIO_OUT1_W1TC: u32 = 0x3FF4_4018;

pub(super) const GPIO_ENABLE_W1TS: u32 = 0x3FF4_4024;

pub(super) const GPIO_ENABLE1_W1TS: u32 = 0x3FF4_4030;

pub(super) const GPIO_IN: u32 = 0x3FF4_403C;

pub(super) const GPIO_IN1: u32 = 0x3FF4_4040;

/// `GPIO_FUNCn_OUT_SEL_CFG_REG` = this + 4n; 0x100 (`SIG_GPIO_OUT_IDX`) routes `GPIO_OUT`.
pub(super) const GPIO_FUNC0_OUT_SEL_CFG: u32 = 0x3FF4_4530;

pub(super) const SIG_GPIO_OUT: i64 = 0x100;

/// `DR_REG_IO_MUX_BASE`, and each pin's register offset from it (`GPIO_PIN_MUX_REG`; 0: no
/// such pin).
pub(super) const IO_MUX_BASE: u32 = 0x3FF4_9000;

pub(super) const IO_MUX_OFFSET: [u8; 40] = [
    0x44, 0x88, 0x40, 0x84, 0x48, 0x6c, 0x60, 0x64, 0x68, 0x54, // 0-9
    0x58, 0x5c, 0x34, 0x38, 0x30, 0x3c, 0x4c, 0x50, 0x70, 0x74, // 10-19
    0x78, 0x7c, 0x80, 0x8c, 0x00, 0x24, 0x28, 0x2c, 0x00, 0x00, // 20-29
    0x00, 0x00, 0x1c, 0x20, 0x14, 0x18, 0x04, 0x08, 0x0c, 0x10, // 30-39
];

/// An IO_MUX register selecting the GPIO function (`MCU_SEL` = `PIN_FUNC_GPIO` 2), default
/// drive (`FUN_DRV` 2) and the input enabled (`FUN_IE`), no pulls.
pub(super) const IO_MUX_GPIO: i64 = (2 << 12) | (2 << 10) | (1 << 9);

/// `RTC_CNTL_CLK_CONF_REG` (`SOC_CLK_SEL`, bits 27-28: 0 crystal, 1 PLL), `DPORT_CPU_PER_CONF_REG`
/// (`CPUPERIOD_SEL`, bits 0-1: 80/160/240 MHz on the PLL) and `APB_CTRL_SYSCLK_CONF_REG`
/// (`PRE_DIV_CNT`, bits 0-9: the crystal's divider).
pub(super) const RTC_CNTL_CLK_CONF: u32 = 0x3FF4_8070;

pub(super) const DPORT_CPU_PER_CONF: u32 = 0x3FF0_003C;

pub(super) const APB_CTRL_SYSCLK_CONF: u32 = 0x3FF6_6000;

/// The GPIO number `n` (a constant), checked for the ESP32: an existing pin, not one of the
/// flash's (6-11), and for writing not an input-only one (34-39).
pub(super) fn gpio_pin(n: Option<f64>, write: bool) -> Result<u32, String> {
    let n =
        n.ok_or("pini ya ubao lazima iwe namba thabiti kwa ESP32 (mfano: ubao_andika(2, kweli))")?;
    let pin = n as u32;
    if n.fract() != 0.0 || !(0.0..40.0).contains(&n) || IO_MUX_OFFSET[pin as usize] == 0 {
        return Err(format!("ESP32 haina pini {n}"));
    }
    if (6..=11).contains(&pin) {
        return Err(format!("pini {pin} ya ESP32 imeunganishwa na flash"));
    }
    if write && pin >= 34 {
        return Err(format!("pini {pin} ya ESP32 ni ya kusoma tu"));
    }
    Ok(pin)
}

/// `TIMG1`'s timer 0 (`DR_REG_TIMERGROUP1_BASE` 0x3FF60000): `T0CONFIG` (bit 31 enable, 30
/// count up, 13-28 the APB clock's divider), the latched count (`T0LO`/`T0HI`, latched by
/// writing bit 31 of `T0UPDATE`, which clears when done) and its reload value (`T0LOADLO`/
/// `T0LOADHI`, applied by writing `T0LOAD`); the timer group's clock and reset bit (15) in
/// `DPORT_PERIP_CLK_EN_REG`/`DPORT_PERIP_RST_EN_REG`.
pub(super) const TIMG1_T0CONFIG: u32 = 0x3FF6_0000;

pub(super) const TIMG1_T0LO: u32 = 0x3FF6_0004;

pub(super) const TIMG1_T0HI: u32 = 0x3FF6_0008;

pub(super) const TIMG1_T0UPDATE: u32 = 0x3FF6_000C;

pub(super) const TIMG1_T0LOADLO: u32 = 0x3FF6_0018;

pub(super) const TIMG1_T0LOADHI: u32 = 0x3FF6_001C;

pub(super) const TIMG1_T0LOAD: u32 = 0x3FF6_0020;

pub(super) const DPORT_PERIP_CLK_EN: u32 = 0x3FF0_00C0;

pub(super) const DPORT_PERIP_RST_EN: u32 = 0x3FF0_00C4;

pub(super) const TIMERGROUP1_BIT: i64 = 1 << 15;

/// Most polls of `T0UPDATE` before reading the count anyway (a model that never clears the
/// bit must not hang the program).
pub(super) const LATCH_POLLS: i64 = 1000;

/// IR for the ESP32's helpers that need control flow: a [`crate::nguvu::ir::Builder`] with the few
/// operations they use.
pub(super) struct Ir(crate::nguvu::ir::Builder);

impl Ir {
    fn new() -> Self {
        Ir(crate::nguvu::ir::Builder::new())
    }

    fn int(&mut self) -> VReg {
        self.0.vreg(Class::Int)
    }

    fn imm(&mut self, op: IntOp, a: VReg, imm: i32) -> VReg {
        let dst = self.int();
        self.0.push(Inst::IntImm { op, dst, a, imm });
        dst
    }

    fn op(&mut self, op: IntOp, a: VReg, b: VReg) -> VReg {
        let dst = self.int();
        self.0.push(Inst::Int { op, dst, a, b });
        dst
    }

    fn cmp_imm(&mut self, cond: ICond, a: VReg, imm: i32) -> VReg {
        let dst = self.int();
        self.0.push(Inst::ICmpImm { cond, dst, a, imm });
        dst
    }

    fn select(&mut self, cond: VReg, a: VReg, b: VReg) -> VReg {
        let dst = self.int();
        self.0.push(Inst::Select { dst, cond, a, b });
        dst
    }

    fn io_load(&mut self, addr: u32) -> VReg {
        let a = self.0.iconst(addr as i64);
        let dst = self.int();
        self.0.push(Inst::IoLoad { dst, addr: a });
        dst
    }

    fn io_store(&mut self, addr: u32, src: VReg) {
        let a = self.0.iconst(addr as i64);
        self.0.push(Inst::IoStore { src, addr: a });
    }

    /// Bits `shift..` of the register at `addr`, masked by `mask`.
    fn field(&mut self, addr: u32, shift: i32, mask: i32) -> VReg {
        let w = self.io_load(addr);
        let s = self.imm(IntOp::Sar, w, shift);
        self.imm(IntOp::And, s, mask)
    }

    /// The helper's 64-bit argument, from its two words ([`crate::nguvu::ir::HELPER_ARGS`]).
    fn arg(&mut self) -> VReg {
        let high = self.imm(IntOp::Shl, VReg(1), 32);
        self.op(IntOp::Or, high, VReg(0))
    }

    fn finish(self) -> Func {
        let mut func = self.0.finish();
        func.args = crate::nguvu::ir::HELPER_ARGS;
        func
    }
}

/// The clock the chip runs: whether the CPU is on the PLL (`SOC_CLK_SEL` 1), its frequency
/// there (80/160/240 MHz by `CPUPERIOD_SEL`), and the crystal's (40 MHz divided by
/// `PRE_DIV_CNT + 1`), which is also the CPU's and the APB's off the PLL.
pub(super) fn chip_clock(ir: &mut Ir) -> (VReg, VReg, VReg) {
    let source = ir.field(RTC_CNTL_CLK_CONF, 27, 3);
    let period = ir.field(DPORT_CPU_PER_CONF, 0, 3);
    let divider = ir.field(APB_CTRL_SYSCLK_CONF, 0, 0x3FF);
    let p1 = ir.imm(IntOp::Add, period, 1);
    let pll = ir.imm(IntOp::Mul, p1, 80);
    let d1 = ir.imm(IntOp::Add, divider, 1);
    let forty = ir.0.iconst(40);
    let xtal = ir.op(IntOp::UDiv, forty, d1);
    let on_pll = ir.cmp_imm(ICond::Eq, source, 1);
    (on_pll, pll, xtal)
}

/// The wait helper: its argument in milliseconds (nothing when not positive), counted on the
/// cycle counter one millisecond at a time (so its 32-bit wrap never matters) at the CPU's
/// clock ([`chip_clock`]).
pub(super) fn board_wait_func() -> Func {
    let mut ir = Ir::new();
    let (entry, test, start, spin, next, done) = (
        ir.0.block(),
        ir.0.block(),
        ir.0.block(),
        ir.0.block(),
        ir.0.block(),
        ir.0.block(),
    );
    ir.0.switch_to(entry);
    let n = ir.arg();
    let (on_pll, pll, xtal) = chip_clock(&mut ir);
    let mhz = ir.select(on_pll, pll, xtal);
    let per_ms = ir.imm(IntOp::Mul, mhz, 1000);
    let zero = ir.0.iconst(0);
    ir.0.terminate(Term::Jump(test));
    // while n > 0
    ir.0.switch_to(test);
    let more = ir.cmp_imm(ICond::Gt, n, 0);
    ir.0.terminate(Term::Branch {
        cond: more,
        then_: start,
        else_: done,
    });
    ir.0.switch_to(start);
    let t0 = ir.int();
    ir.0.push(Inst::CycleCount { dst: t0 });
    ir.0.terminate(Term::Jump(spin));
    // until (ccount - t0) mod 2^32 reaches a millisecond's cycles
    ir.0.switch_to(spin);
    let now = ir.int();
    ir.0.push(Inst::CycleCount { dst: now });
    let diff = ir.op(IntOp::Sub, now, t0);
    let low = ir.0.iconst(0xFFFF_FFFF);
    let elapsed = ir.op(IntOp::And, diff, low);
    let waiting = ir.int();
    ir.0.push(Inst::ICmp {
        cond: ICond::Ult,
        dst: waiting,
        a: elapsed,
        b: per_ms,
    });
    ir.0.terminate(Term::Branch {
        cond: waiting,
        then_: spin,
        else_: next,
    });
    ir.0.switch_to(next);
    ir.0.push(Inst::IntImm {
        op: IntOp::Sub,
        dst: n,
        a: n,
        imm: 1,
    });
    ir.0.terminate(Term::Jump(test));
    ir.0.switch_to(done);
    ir.0.terminate(Term::Return(zero));
    ir.finish()
}

/// The clock helper: whole milliseconds on `TIMG1`'s timer 0, counting microseconds (the APB
/// clock divided by its MHz). The timer holds the state: stopped means not started yet, so the
/// first call starts it at 0 — and so does an argument other than 0 (the entry point, so a
/// program's clock starts with it).
pub(super) fn board_clock_func() -> Func {
    let mut ir = Ir::new();
    let (entry, start, read, poll, done) = (
        ir.0.block(),
        ir.0.block(),
        ir.0.block(),
        ir.0.block(),
        ir.0.block(),
    );
    ir.0.switch_to(entry);
    let restart = ir.arg();
    let enabled = ir.field(TIMG1_T0CONFIG, 31, 1);
    let stopped = ir.cmp_imm(ICond::Eq, enabled, 0);
    let asked = ir.cmp_imm(ICond::Ne, restart, 0);
    let fresh = ir.op(IntOp::Or, stopped, asked);
    ir.0.terminate(Term::Branch {
        cond: fresh,
        then_: start,
        else_: read,
    });
    // Start: the group's clock on and out of reset, the timer stopped, loaded with 0, started.
    ir.0.switch_to(start);
    let clk = ir.io_load(DPORT_PERIP_CLK_EN);
    let clk_on = ir.imm(IntOp::Or, clk, TIMERGROUP1_BIT as i32);
    ir.io_store(DPORT_PERIP_CLK_EN, clk_on);
    let rst = ir.io_load(DPORT_PERIP_RST_EN);
    let rst_off = ir.imm(IntOp::And, rst, !(TIMERGROUP1_BIT as i32));
    ir.io_store(DPORT_PERIP_RST_EN, rst_off);
    let (on_pll, _, xtal) = chip_clock(&mut ir);
    let eighty = ir.0.iconst(80);
    let apb_mhz = ir.select(on_pll, eighty, xtal);
    let divider = ir.imm(IntOp::Shl, apb_mhz, 13);
    let counting_up = ir.imm(IntOp::Or, divider, 1 << 30);
    ir.io_store(TIMG1_T0CONFIG, counting_up);
    let zero = ir.0.iconst(0);
    ir.io_store(TIMG1_T0LOADLO, zero);
    ir.io_store(TIMG1_T0LOADHI, zero);
    let one = ir.0.iconst(1);
    ir.io_store(TIMG1_T0LOAD, one);
    let running = ir.imm(IntOp::Or, counting_up, i32::MIN); // bit 31
    ir.io_store(TIMG1_T0CONFIG, running);
    ir.0.terminate(Term::Jump(read));
    // Latch the count (the timer runs in another clock domain), then read it.
    ir.0.switch_to(read);
    let latch = ir.0.iconst(1 << 31);
    ir.io_store(TIMG1_T0UPDATE, latch);
    let tries = ir.0.iconst(LATCH_POLLS);
    ir.0.terminate(Term::Jump(poll));
    ir.0.switch_to(poll);
    let busy = ir.field(TIMG1_T0UPDATE, 31, 1);
    ir.0.push(Inst::IntImm {
        op: IntOp::Sub,
        dst: tries,
        a: tries,
        imm: 1,
    });
    let left = ir.cmp_imm(ICond::Gt, tries, 0);
    let again = ir.op(IntOp::And, busy, left);
    ir.0.terminate(Term::Branch {
        cond: again,
        then_: poll,
        else_: done,
    });
    ir.0.switch_to(done);
    let lo = ir.io_load(TIMG1_T0LO);
    let hi = ir.io_load(TIMG1_T0HI);
    let high = ir.imm(IntOp::Shl, hi, 32);
    let us = ir.op(IntOp::Or, high, lo);
    let thousand = ir.0.iconst(1000);
    let ms = ir.op(IntOp::UDiv, us, thousand);
    ir.0.terminate(Term::Return(ms));
    ir.finish()
}

/// Replace the device runtime's small calls with IR, so the ESP32 needs no runtime library of its
/// own: list access becomes loads from the caller's list references (`{ double *data; uint32_t
/// len }`, eight bytes on these 32-bit devices) and the host-only calls their failing status. The
/// float conversions stay calls, to helpers compiled from IR once per object
/// ([`helper_func`]); only `fmod` and `pow` come from the C library. Each computes what
/// `driver/kifaa` computes, which is what the host's runtime computes.
pub(super) fn inline_runtime(func: &mut Func) -> Result<(), String> {
    let consts = iconsts(func);
    // Registers defined once by a constant, as numbers (a pin must be one).
    let numbers: std::collections::HashMap<u32, f64> = func
        .homes()
        .iter()
        .enumerate()
        .filter_map(|(v, h)| match h {
            crate::nguvu::ir::Home::Const(bits) => Some((
                v as u32,
                match func.classes[v] {
                    Class::Int => *bits as f64,
                    Class::Float => f64::from_bits(*bits as u64),
                },
            )),
            _ => None,
        })
        .collect();
    let mut classes = std::mem::take(&mut func.classes);
    for block in &mut func.blocks {
        let mut out = Vec::with_capacity(block.insts.len());
        for inst in std::mem::take(&mut block.insts) {
            let Inst::Call {
                target, args, dst, ..
            } = &inst
            else {
                out.push(inst);
                continue;
            };
            let mut e = Emit {
                classes: &mut classes,
                out: &mut out,
            };
            match (target, dst) {
                (RtFn::Exec, Some(d)) => e.push(Inst::IConst {
                    dst: *d,
                    value: EXEC_FAIL,
                }),
                (RtFn::CallHost | RtFn::DepthError, Some(d)) => e.push(Inst::IConst {
                    dst: *d,
                    value: STATUS_FAIL,
                }),
                (RtFn::Exec | RtFn::CallHost | RtFn::DepthError, None) => {}
                (RtFn::ListPtr | RtFn::ListLen, Some(d)) => {
                    let reg = args
                        .get(1)
                        .and_then(|r| consts.get(&r.0))
                        .ok_or("kosa la ndani: rejista ya orodha si thabiti")?;
                    let offset =
                        i32::try_from(8 * reg).map_err(|_| "kosa la ndani: orodha nyingi mno")?;
                    // Both words of the reference: data pointer low, length high.
                    let both = e.int();
                    e.push(Inst::Load {
                        dst: both,
                        base: args[0],
                        offset,
                    });
                    let word = if *target == RtFn::ListLen {
                        let w = e.int();
                        e.push(Inst::IntImm {
                            op: IntOp::Sar,
                            dst: w,
                            a: both,
                            imm: 32,
                        });
                        w
                    } else {
                        both
                    };
                    let mask = e.iconst(0xFFFF_FFFF);
                    e.push(Inst::Int {
                        op: IntOp::And,
                        dst: *d,
                        a: word,
                        b: mask,
                    });
                }
                (RtFn::PinWrite, None) => {
                    let n = gpio_pin(numbers.get(&args[0].0).copied(), true)?;
                    e.pin_write(n, args[1]);
                }
                (RtFn::PinRead, Some(d)) => {
                    let n = gpio_pin(numbers.get(&args[0].0).copied(), false)?;
                    e.pin_read(n, *d);
                }
                _ => out.push(inst),
            }
        }
        block.insts = out;
    }
    func.classes = classes;
    Ok(())
}

/// The IR function for helper `target` (one of [`codegen_xt::HELPERS`]): its 64-bit argument
/// arrives as two words ([`crate::nguvu::ir::HELPER_ARGS`]), its 64-bit result (a double's bits or an
/// integer) is returned.
pub(super) fn helper_func(target: RtFn) -> Func {
    match target {
        RtFn::BoardWait => return board_wait_func(),
        RtFn::BoardClock => return board_clock_func(),
        _ => {}
    }
    let mut classes = vec![Class::Int, Class::Int];
    let mut out = Vec::new();
    let mut e = Emit {
        classes: &mut classes,
        out: &mut out,
    };
    let (lo, hi) = (VReg(0), VReg(1));
    let high = e.int();
    e.push(Inst::IntImm {
        op: IntOp::Shl,
        dst: high,
        a: hi,
        imm: 32,
    });
    let bits = e.int();
    e.push(Inst::Int {
        op: IntOp::Or,
        dst: bits,
        a: high,
        b: lo,
    });
    let x = e.float();
    e.push(Inst::BitsFloat { dst: x, src: bits });
    let result = match target {
        RtFn::Floor | RtFn::Ceil => {
            let r = e.floor_ceil(x, target == RtFn::Floor);
            let b = e.int();
            e.push(Inst::FloatBits { dst: b, src: r });
            b
        }
        RtFn::FloatToIntSat => e.float_to_int_sat(x),
        _ => e.shift_amount(x),
    };
    Func {
        classes,
        blocks: vec![BlockData {
            insts: out,
            term: Term::Return(result),
        }],
        cold: vec![false],
        call_buffer: 0,
        args: crate::nguvu::ir::HELPER_ARGS,
    }
}

/// Appends instructions (and new registers) for [`inline_runtime`] and [`helper_func`].
pub(super) struct Emit<'a> {
    classes: &'a mut Vec<Class>,
    out: &'a mut Vec<Inst>,
}

impl Emit<'_> {
    fn push(&mut self, inst: Inst) {
        self.out.push(inst);
    }

    /// `[addr] = value`, a peripheral register.
    fn io_store(&mut self, addr: u32, value: VReg) {
        let a = self.iconst(addr as i64);
        self.push(Inst::IoStore {
            src: value,
            addr: a,
        });
    }

    /// GPIO `n` driven to `level != 0`: the pin in GPIO function, routed from `GPIO_OUT` and
    /// enabled as an output (each time: idempotent, and no state to keep), then set or cleared.
    fn pin_write(&mut self, n: u32, level: VReg) {
        let mux = self.iconst(IO_MUX_GPIO);
        self.io_store(IO_MUX_BASE + IO_MUX_OFFSET[n as usize] as u32, mux);
        let sel = self.iconst(SIG_GPIO_OUT);
        self.io_store(GPIO_FUNC0_OUT_SEL_CFG + 4 * n, sel);
        let (enable, set, clear) = if n < 32 {
            (GPIO_ENABLE_W1TS, GPIO_OUT_W1TS, GPIO_OUT_W1TC)
        } else {
            (GPIO_ENABLE1_W1TS, GPIO_OUT1_W1TS, GPIO_OUT1_W1TC)
        };
        let bit = self.iconst(1 << (n % 32));
        self.io_store(enable, bit);
        let on = self.int();
        self.push(Inst::ICmpImm {
            cond: ICond::Ne,
            dst: on,
            a: level,
            imm: 0,
        });
        let set = self.iconst(set as i64);
        let clear = self.iconst(clear as i64);
        let addr = self.int();
        self.push(Inst::Select {
            dst: addr,
            cond: on,
            a: set,
            b: clear,
        });
        self.push(Inst::IoStore { src: bit, addr });
    }

    /// `dst` = GPIO `n`'s level (0/1): the pin in GPIO function with its input enabled, then its
    /// bit of `GPIO_IN`.
    fn pin_read(&mut self, n: u32, dst: VReg) {
        let mux = self.iconst(IO_MUX_GPIO);
        self.io_store(IO_MUX_BASE + IO_MUX_OFFSET[n as usize] as u32, mux);
        let a = self.iconst(if n < 32 { GPIO_IN } else { GPIO_IN1 } as i64);
        let word = self.int();
        self.push(Inst::IoLoad { dst: word, addr: a });
        let shifted = self.int();
        self.push(Inst::IntImm {
            op: IntOp::Sar,
            dst: shifted,
            a: word,
            imm: (n % 32) as i32,
        });
        self.push(Inst::IntImm {
            op: IntOp::And,
            dst,
            a: shifted,
            imm: 1,
        });
    }

    fn fresh(&mut self, class: Class) -> VReg {
        self.classes.push(class);
        VReg(self.classes.len() as u32 - 1)
    }

    fn int(&mut self) -> VReg {
        self.fresh(Class::Int)
    }

    fn float(&mut self) -> VReg {
        self.fresh(Class::Float)
    }

    fn iconst(&mut self, value: i64) -> VReg {
        let dst = self.int();
        self.push(Inst::IConst { dst, value });
        dst
    }

    fn fconst(&mut self, value: f64) -> VReg {
        let dst = self.float();
        self.push(Inst::FConst { dst, value });
        dst
    }

    fn fcmp(&mut self, cond: FCond, a: VReg, b: VReg) -> VReg {
        let dst = self.int();
        self.push(Inst::FCmp { cond, dst, a, b });
        dst
    }

    fn select_int(&mut self, cond: VReg, a: VReg, b: VReg) -> VReg {
        let dst = self.int();
        self.push(Inst::Select { dst, cond, a, b });
        dst
    }

    /// `floor(x)` or `ceil(x)`: `t - 1` for a negative non-integer, `t + 1` for a positive one,
    /// with `t = trunc(x)` — exact, since |t| < 2^52 whenever x is not an integer.
    fn floor_ceil(&mut self, x: VReg, floor: bool) -> VReg {
        let t = self.trunc(x);
        let zero = self.fconst(0.0);
        let one = self.fconst(1.0);
        let (cond, op) = if floor {
            (FCond::Olt, FloatOp::Sub)
        } else {
            (FCond::Ogt, FloatOp::Add)
        };
        let side = self.fcmp(cond, x, zero);
        let inexact = self.fcmp(FCond::Une, t, x);
        let step = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: step,
            a: side,
            b: inexact,
        });
        let moved = self.float();
        self.push(Inst::Float {
            op,
            dst: moved,
            a: t,
            b: one,
        });
        let r = self.float();
        self.push(Inst::Select {
            dst: r,
            cond: step,
            a: moved,
            b: t,
        });
        r
    }

    /// Rust's saturating `x as i64`: NaN -> 0, beyond the range -> MIN / MAX.
    fn float_to_int_sat(&mut self, x: VReg) -> VReg {
        let t = self.trunc(x);
        let i = self.int();
        self.push(Inst::FloatToInt { dst: i, src: t });
        let limit = self.fconst(9223372036854775808.0);
        let low = self.fconst(-9223372036854775808.0);
        let above = self.fcmp(FCond::Oge, x, limit);
        let below = self.fcmp(FCond::Ole, x, low);
        let nan = self.fcmp(FCond::Une, x, x);
        let max = self.iconst(i64::MAX);
        let min = self.iconst(i64::MIN);
        let zero = self.iconst(0);
        let r1 = self.select_int(above, max, i);
        let r2 = self.select_int(below, min, r1);
        self.select_int(nan, zero, r2)
    }

    /// The language's shift amount: `x as i32` (saturating, NaN -> 0) kept only within 0..=63,
    /// which is exactly the x with 0 <= x < 64, truncated.
    fn shift_amount(&mut self, x: VReg) -> VReg {
        let t = self.trunc(x);
        let i = self.int();
        self.push(Inst::FloatToInt { dst: i, src: t });
        let zero_f = self.fconst(0.0);
        let sixty_four = self.fconst(64.0);
        let ge = self.fcmp(FCond::Oge, x, zero_f);
        let lt = self.fcmp(FCond::Olt, x, sixty_four);
        let inside = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: inside,
            a: ge,
            b: lt,
        });
        let zero = self.iconst(0);
        self.select_int(inside, i, zero)
    }

    /// `trunc(x)`, exactly, from the bits: an exponent below 0 leaves a signed zero, one of 52
    /// or more (integers, infinities, NaN) leaves `x`, anything between clears the fraction bits
    /// below the binary point.
    fn trunc(&mut self, x: VReg) -> VReg {
        let bits = self.int();
        self.push(Inst::FloatBits { dst: bits, src: x });
        let shifted = self.int();
        self.push(Inst::IntImm {
            op: IntOp::Sar,
            dst: shifted,
            a: bits,
            imm: 52,
        });
        let field = self.int();
        self.push(Inst::IntImm {
            op: IntOp::And,
            dst: field,
            a: shifted,
            imm: 0x7ff,
        });
        let e = self.int();
        self.push(Inst::IntImm {
            op: IntOp::Sub,
            dst: e,
            a: field,
            imm: 1023,
        });
        let small = self.int();
        self.push(Inst::ICmpImm {
            cond: ICond::Lt,
            dst: small,
            a: e,
            imm: 0,
        });
        let whole = self.int();
        self.push(Inst::ICmpImm {
            cond: ICond::Gt,
            dst: whole,
            a: e,
            imm: 51,
        });
        // The shift amount, kept within 0..=51 on every path (only one is selected).
        let zero = self.iconst(0);
        let fifty_one = self.iconst(51);
        let capped = self.select_int(whole, fifty_one, e);
        let amount = self.select_int(small, zero, capped);
        let fraction = self.iconst(0x000F_FFFF_FFFF_FFFF);
        let below = self.int();
        self.push(Inst::Int {
            op: IntOp::Sar,
            dst: below,
            a: fraction,
            b: amount,
        });
        let keep = self.int();
        self.push(Inst::Not {
            dst: keep,
            src: below,
        });
        let cleared = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: cleared,
            a: bits,
            b: keep,
        });
        let sign_mask = self.iconst(i64::MIN);
        let sign = self.int();
        self.push(Inst::Int {
            op: IntOp::And,
            dst: sign,
            a: bits,
            b: sign_mask,
        });
        let r1 = self.select_int(whole, bits, cleared);
        let r2 = self.select_int(small, sign, r1);
        let t = self.float();
        self.push(Inst::BitsFloat { dst: t, src: r2 });
        t
    }
}
