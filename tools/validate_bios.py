"""Local BIOS/core baseline. Requires user-supplied ROMs/discs; distributes none.

Run after: cargo build --release --bin harness --bin bios_audit --bin bios_contract
Only writes to --output. Never loads/saves the user's NVRAM or app settings.
Results test integrity and repeatability, not equivalence to original hardware.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bios-dir', type=Path, required=True)
    p.add_argument('--disc-dir', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--bin-dir', type=Path, default=Path(__file__).resolve().parents[1] / 'target/release')
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    suffix = '.exe' if os.name == 'nt' else ''
    harness, audit = (a.bin_dir / (name + suffix) for name in ['harness', 'bios_audit'])
    # Pin each input by content identity; game/firmware bytes stay outside results.
    inputs = {}
    def identity(path):
        if path.name not in inputs:
            h = hashlib.sha256()
            with path.open('rb') as f:
                for chunk in iter(lambda: f.read(1024 * 1024), b''): h.update(chunk)
            inputs[path.name] = {'sha256': h.hexdigest(), 'size': path.stat().st_size}

    results = []
    env = os.environ.copy()
    for key in ['CDI_OS9_TRACE', 'CDI_DC_TRACE', 'CDI_IO_HASHES']:
        env.pop(key, None)

    def run(name, command, expected=0, trace=False):
        case_env = env.copy()
        if trace: case_env['CDI_OS9_TRACE'] = str((a.output / (name + '.trace')).resolve())
        started = time.perf_counter()
        try:
            proc = subprocess.run([str(x) for x in command], env=case_env, capture_output=True,
                                  timeout=180, encoding='utf-8', errors='replace')
            text = proc.stdout + proc.stderr
            code = proc.returncode
        except subprocess.TimeoutExpired:
            code, text = -1, 'TIMEOUT after 180 seconds\n'
        (a.output / (name + '.log')).write_text(text, encoding='utf-8')
        result = {'name': name, 'exit_code': code, 'expected_exit': expected,
                  'passed': code == expected, 'seconds': round(time.perf_counter() - started, 3),
                  'observations': [line for line in text.splitlines() if line.startswith(
                      ('HARNESS', 'ROUNDTRIP ', 'ROUNDTRIP-RESAVE', 'AUTOPLAY ', 'SUMMARY', 'COUNTERS'))]}
        if trace:
            counts, errors = {}, {}
            trace_path = a.output / (name + '.trace')
            if trace_path.exists():
                with trace_path.open(encoding='utf-8') as f:
                    for line in f:
                        if line.startswith('C '):
                            m = re.search(r't=(\d+) code=([0-9A-F]+)', line)
                            if m:
                                k = 'trap' + m[1] + ':' + m[2]
                                counts[k] = counts.get(k, 0) + 1
                        elif line.startswith('R ') and ' cc=1 ' in line:
                            m = re.search(r'err=([0-9A-F]+)', line)
                            if m: errors[m[1]] = errors.get(m[1], 0) + 1
                result['trace'] = {'calls': counts, 'error_returns': errors,
                                   'note': 'Error returns can be expected feature/file probes; not automatic bugs.'}
        results.append(result)
        (a.output / 'results.json').write_text(json.dumps({'inputs': inputs, 'results': results}, indent=2), encoding='utf-8')
        print(name, 'PASS' if result['passed'] else 'FAIL', result['seconds'], flush=True)
        return text

    roms = [a.bios_dir / x for x in ['cdi220b.rom', 'cdi490a.rom', 'vmpega.rom', 'impega.rom']]
    for rom in roms: identity(rom)
    run('module-inventory', [audit, *roms])
    system, vmpeg, impeg = roms[0], roms[2], roms[3]
    run('220-crc-contract', [a.bin_dir / ('bios_contract' + suffix), system])
    cases = [
        ('220-shell-pal', system, 'none', None, False),
        ('220-shell-ntsc', system, 'none', None, True),
        ('220-vmpeg-pal', system, 'none', vmpeg, False),
        ('490-impeg-pal', roms[1], 'none', impeg, False),
        ('third-degree-pal', system, '3rd Degree (USA).chd', vmpeg, False),
        ('maddog-ntsc', system, 'Mad Dog McCree (USA, Europe).chd', vmpeg, True),
        ('chaos-pal', system, 'Chaos Control (USA).chd', vmpeg, False),
    ]
    for name, bios, disc, dvc, ntsc in cases:
        disc_path = 'none' if disc == 'none' else a.disc_dir / disc
        if disc != 'none': identity(disc_path)
        command = [harness, bios, disc_path, '1200', '--roundtrip', '900', '--counters']
        if dvc: command += ['--dvc', dvc]
        if ntsc: command += ['--ntsc']
        if disc != 'none': command += ['--require-launch']
        run(name, command)

    # A tracer must not change the observed core output. Use a separate no-replay
    # pair; tracing can be slower and is deliberately kept out of timing claims.
    base = [harness, system, 'none', '700', '--dvc', vmpeg, '--counters']
    plain = run('trace-control-off', base)
    traced = run('trace-control-on', base, trace=True)
    signature = lambda text: [line for line in text.splitlines() if line.startswith(('HARNESS', 'COUNTERS'))]
    same = bool(signature(plain)) and signature(plain) == signature(traced)
    results.append({'name': 'trace-observer-parity', 'passed': same})
    # Verify that a zero-frame launch test is rejected instead of passing silently.
    run('required-launch-negative', [harness, system, a.disc_dir / '3rd Degree (USA).chd', '0', '--require-launch'], expected=1)
    (a.output / 'results.json').write_text(json.dumps({'inputs': inputs, 'results': results}, indent=2), encoding='utf-8')
    if not all(x['passed'] for x in results): raise SystemExit(1)


if __name__ == '__main__':
    main()
