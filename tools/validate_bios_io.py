"""Verify four real BIOS read results during a Third Degree boot, using hashes.

Build harness, disc_data_audit and bios_io_contract first. All evidence stays in
--output; user saves/settings are never loaded or written. No content is exported.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ['bios', 'dvc', 'disc', 'output']:
        parser.add_argument('--' + arg, type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=Path(__file__).resolve().parents[1] / 'target/release')
    a = parser.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    ext = '.exe' if os.name == 'nt' else ''
    env = os.environ.copy()
    for key in ['CDI_OS9_TRACE', 'CDI_DC_TRACE', 'CDI_IO_HASHES']: env.pop(key, None)
    def run(name, command, overrides=None):
        child_env = env.copy()
        if overrides: child_env.update(overrides)
        p = subprocess.run([str(x) for x in command], capture_output=True, encoding='utf-8',
                           errors='replace', timeout=180, env=child_env)
        (a.output / (name + '.log')).write_text(p.stdout + p.stderr, encoding='utf-8')
        if p.returncode: raise RuntimeError(name + ' failed; see its log')
        return p.stdout

    run('pointer-contract', [a.bin_dir / ('bios_io_contract' + ext), a.bios])
    manifest = run('disc-data', [a.bin_dir / ('disc_data_audit' + ext), a.disc])
    expected = {}
    required = {'cdi_hotseat_loader', 'cdi_hotseat_attract', 'cdi_bumper'}
    for line in manifest.splitlines():
        fields = line.split('\t')
        if fields[0] == 'LABEL': expected['volume descriptor'] = (2048, fields[-1])
        elif fields[0] == 'FILE' and fields[1] in required:
            expected[fields[1]] = (int(fields[2]), fields[3])
    if len(expected) != 4: raise RuntimeError('This fixture requires the Third Degree disc and three named modules')
    trace = (a.output / 'disc-reads.trace').resolve()
    run('normal-boot', [a.bin_dir / ('harness' + ext), a.bios, a.disc, '1300', '--dvc', a.dvc, '--require-launch'],
        {'CDI_OS9_TRACE': str(trace), 'CDI_IO_HASHES': '1'})
    reads = []
    with trace.open(encoding='utf-8') as f:
        for line in f:
            if not line.startswith('R '): continue
            m = re.search(r'read_count=(\d+) read_sha1=([0-9a-f]{40})', line)
            if m: reads.append((int(m[1]), m[2]))
    checks = [{'name': name, 'size': size, 'sha1': digest, 'passed': (size, digest) in reads}
              for name, (size, digest) in expected.items()]
    result = {'checks': checks, 'successful_reads_observed': len(reads),
              'scope': 'Four whole-buffer results in one normal boot; not all reads, filesystem errors, or timing conformance.'}
    (a.output / 'results.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    for check in checks: print(check['name'], 'PASS' if check['passed'] else 'FAIL', flush=True)
    if not all(x['passed'] for x in checks): raise SystemExit(1)


if __name__ == '__main__':
    main()
