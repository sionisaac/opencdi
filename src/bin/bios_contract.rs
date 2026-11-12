//! Focused OS-9 F$CRC contract probe against a locally supplied BIOS.
//! Runs firmware in OpenCDi, then calls its service with synthetic data. This
//! checks one observable contract, not original-hardware timing or full CPU accuracy.
use m68k::{CycleBatchExit, StepResult};
use opencdi::{bios_audit::crc24, machine::Machine};

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        eprintln!("usage: bios_contract <cdi220b.rom|cdi490a.rom>");
        std::process::exit(2);
    }
    let mut m = Machine::new();
    m.bus.nvram.persist = false;
    let report = m.load_bios(&args[0]).expect("load BIOS");
    if report.known.is_none_or(|r| !matches!(r.file, "cdi220b.rom" | "cdi490a.rom")) {
        eprintln!("probe requires an identified CD-i 220 F2 or 490 BIOS");
        std::process::exit(2);
    }
    m.reset();
    for _ in 0..700 {
        m.run_frame();
    }
    let mut state = Vec::new();
    m.save_state(&[], &mut state);
    let patterns: Vec<Vec<u8>> = vec![
        vec![],
        vec![0],
        vec![0xff],
        b"123456789".to_vec(),
        (0..=255).collect(),
        (0..4096).map(|n| ((n * 73 + 19) & 255) as u8).collect(),
    ];
    let mut passed = 0;
    for batched in [false, true] {
        for data in &patterns {
            for initial in [0xffffff, 0, 0x123456] {
                m.load_state(&state).expect("restore isolated probe");
                // Scratch space is private to this disposable headless machine.
                // TRAP #0; F$CRC ($17); return marker (BRA -2, never executed).
                m.bus.mcd212.plane_a[0x70000..0x70006]
                    .copy_from_slice(&[0x4e, 0x40, 0, 0x17, 0x60, 0xfe]);
                m.bus.mcd212.plane_a[0x71000..0x71000 + data.len()].copy_from_slice(data);
                m.cpu.set_sr(0x2700);
                m.cpu.set_irq(0);
                m.cpu.dar[15] = 0x7f000;
                m.cpu.dar[0] = data.len() as u32;
                m.cpu.dar[1] = initial;
                m.cpu.dar[8] = 0x71000;
                m.cpu.stopped = 0;
                m.cpu.jump(0x70000);
                let mut returned = false;
                for _ in 0..200_000 {
                    if m.cpu.pc == 0x70004 {
                        returned = true;
                        break;
                    }
                    if batched {
                        // Same CPU batch size used by Machine::run_frame. The marker
                        // loops without changing registers if a batch reaches it.
                        match m.cpu.run_for_cycles(&mut m.bus, 32).exit {
                            CycleBatchExit::BudgetExhausted => {}
                            CycleBatchExit::TrapInstruction { trap_num } => {
                                m.cpu.take_trap_exception(&mut m.bus, trap_num);
                            }
                            other => panic!("unexpected batch exit at {:08x}: {other:?}", m.cpu.pc),
                        }
                    } else {
                        match m.cpu.step(&mut m.bus) {
                            StepResult::Ok { .. } => {}
                            StepResult::TrapInstruction { trap_num } => {
                                m.cpu.take_trap_exception(&mut m.bus, trap_num);
                            }
                            other => panic!("unexpected CPU exit at {:08x}: {other:?}", m.cpu.pc),
                        }
                    }
                }
                let expected = crc24(data, initial);
                let actual = m.cpu.dar[1] & 0xffffff;
                let ok = returned && m.cpu.get_ccr() & 1 == 0 && actual == expected;
                println!(
                    "FCRC batched={batched} length={} initial={initial:06x} expected={expected:06x} actual={actual:06x} returned={returned} carry={} {}",
                    data.len(),
                    m.cpu.get_ccr() & 1,
                    if ok { "PASS" } else { "FAIL" }
                );
                if !ok {
                    std::process::exit(1);
                }
                passed += 1;
            }
        }
    }
    println!("BIOS_CONTRACT FCRC cases={passed} passed={passed}");
}
