"""Validate BIOS file seeks, EOF and errors in a disposable Third Degree process.

Requires: cargo build --release --bin bios_file_contract --bin disc_data_audit
Outputs hashes and metadata only. Input ROMs and disc must be supplied locally.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''): h.update(block)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['bios', 'dvc', 'disc', 'output']: p.add_argument('--' + name, type=Path, required=True)
    p.add_argument('--bin-dir', type=Path, default=Path(__file__).resolve().parents[1] / 'target/release')
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    ext = '.exe' if os.name == 'nt' else ''
    env = os.environ.copy()
    for key in ['CDI_OS9_TRACE', 'CDI_DC_TRACE', 'CDI_IO_HASHES']: env.pop(key, None)
    identities = {path.name: digest(path) for path in [a.bios, a.dvc, a.disc]}
    def run(name, command):
        result = subprocess.run([str(x) for x in command], capture_output=True, encoding='utf-8',
                                errors='replace', timeout=180, env=env)
        (a.output / (name + '.log')).write_text(result.stdout + result.stderr, encoding='utf-8')
        if result.returncode: raise RuntimeError(name + ' failed; see the log')
        return result.stdout

    manifest = run('reference', [a.bin_dir / ('disc_data_audit' + ext), a.disc, '--contract-slices'])
    references = {}
    for line in manifest.splitlines():
        f = line.split('\t')
        if f[0] == 'SLICE':
            key = (f[1], int(f[2]), int(f[3]))
            if key in references and references[key] != f[4]: raise RuntimeError('ambiguous reference')
            references[key] = f[4]
    checks = []
    service_checks = 0
    streams = []
    buffers = []
    variance = []
    summaries_pass = True
    for mode, options in [('Pal', []), ('Ntsc', ['--ntsc'])]:
        observed = run('file-contract-' + mode.lower(),
                       [a.bin_dir / ('bios_file_contract' + ext), a.bios, a.dvc, a.disc, *options])
        for line in observed.splitlines():
            f = line.split('\t')
            if f[0] == 'READ':
                key = (f[1], int(f[2]), int(f[3]))
                checks.append({'mode': mode, 'file': key[0], 'offset': key[1], 'count': key[2],
                               'sha1': f[4], 'passed': references.get(key) == f[4]})
        summary = re.search(r'BIOS_FILE_CONTRACT mode=' + mode + r' checks=(\d+) passed=(\d+)', observed)
        summaries_pass &= bool(summary and summary[1] == summary[2] == '46')
        if summary: service_checks += int(summary[1])
        variance.extend({'mode': mode, 'caller_d1': int(caller, 16), 'driver_d1': int(driver, 16)}
                        for caller, driver in re.findall(r'EOF_PRECISE ok=true d1=([0-9a-f]+) driver_d1=([0-9a-f]+)', observed))
        stream = run('stream-contract-' + mode.lower(),
                     [a.bin_dir / ('bios_file_contract' + ext), a.bios, a.dvc, a.disc, '--stream', *options])
        summary = re.search(r'BIOS_STREAM_CONTRACT mode=' + mode + r' checks=(\d+) passed=(\d+)', stream)
        aborts = re.findall(r'ABORT status=([0-9a-f]+) signal=([0-9a-f]+) stopped=true', stream)
        passed = bool(summary and summary[1] == summary[2] == '21' and len(aborts) == 2)
        passed &= all(int(status,16) & 0x8100 == 0x8100 and int(signal,16) == 0x060a for status,signal in aborts)
        streams.append({'mode': mode, 'checks': int(summary[1]) if summary else 0, 'passed': passed,
                        'aborts': [{'status': int(s,16), 'signal': int(sig,16)} for s,sig in aborts]})
        print('Streaming', mode, 'PASS' if passed else 'FAIL', flush=True)
        buffered = run('buffer-contract-' + mode.lower(),
                       [a.bin_dir / ('bios_file_contract' + ext), a.bios, a.dvc, a.disc, '--buffers', *options])
        summary = re.search(r'BIOS_BUFFER_CONTRACT mode=' + mode + r' checks=(\d+) passed=(\d+)', buffered)
        sectors = re.findall(r'BUFFER mode=' + mode + r' index=(\d+) lba=(\d+) channel=(\d+) bytes=(\d+) sha1=([0-9a-f]{40}) PASS', buffered)
        passed = bool(summary and summary[1] == summary[2] == '14' and len(sectors) == 2)
        buffers.append({'mode': mode, 'checks': int(summary[1]) if summary else 0, 'passed': passed,
                        'sectors': [{'index': int(i), 'lba': int(lba), 'channel': int(ch), 'bytes': int(n), 'sha1': sha}
                                    for i,lba,ch,n,sha in sectors]})
        print('Linked buffers', mode, 'PASS' if passed else 'FAIL', flush=True)
    buffer_parity = len(buffers) == 2 and buffers[0]['sectors'] == buffers[1]['sectors']
    unchanged = all(identities[path.name] == digest(path) for path in [a.bios, a.dvc, a.disc])
    results = {'inputs_sha256': identities, 'inputs_unchanged': unchanged,
               'service_checks': service_checks, 'read_checks': checks,
               'stream_results': streams,
               'buffer_results': buffers, 'buffer_mode_parity': buffer_parity,
               'non_eof_observations': variance,
               'scope': 'One BIOS/disc in PAL and NTSC; precise/batched EOF agreement, not hardware timing or every filesystem case.'}
    (a.output / 'results.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
    for check in checks: print(check['mode'], check['file'], check['offset'], check['count'], 'PASS' if check['passed'] else 'FAIL', flush=True)
    print('Input images unchanged:', unchanged, flush=True)
    if not summaries_pass or len(checks) != 14 or not all(c['passed'] for c in checks) or not all(s['passed'] for s in streams) or not all(b['passed'] for b in buffers) or not buffer_parity or not unchanged:
        raise SystemExit(1)


if __name__ == '__main__': main()
