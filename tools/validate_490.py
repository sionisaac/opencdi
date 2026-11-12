"""Validate identified 490/IMPEG firmware with disposable machines and local discs.

Build bios_contract, bios_io_contract, bios_490_contract and disc_data_audit first.
Writes status/hash metadata to --output, never firmware or game contents.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def digest(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''): h.update(block)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for key in ['bios', 'dvc', 'disc', 'output']: p.add_argument('--' + key, type=Path, required=True)
    p.add_argument('--bin-dir', type=Path, default=Path(__file__).resolve().parents[1] / 'target/release')
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    ext = '.exe' if os.name == 'nt' else ''
    identities = {str(f): digest(f) for f in [a.bios, a.dvc, a.disc]}
    env = os.environ.copy()
    for key in ['CDI_OS9_TRACE', 'CDI_DC_TRACE', 'CDI_IO_HASHES', 'CDI_BUS_FAULT_TRACE']: env.pop(key, None)

    def run(name, binary, args):
        result = subprocess.run([str(a.bin_dir / (binary + ext)), *map(str,args)],
                                capture_output=True, encoding='utf-8', errors='replace', timeout=180, env=env)
        (a.output / (name + '.log')).write_text(result.stdout + result.stderr, encoding='utf-8')
        if result.returncode: raise RuntimeError(name + ' failed; see log')
        return result.stdout

    reference = run('disc-reference', 'disc_data_audit', [a.disc])
    hashes = {f[1]: (int(f[2]), f[3]) for line in reference.splitlines()
              if (f := line.split('\t'))[0] == 'FILE'}
    crc = run('crc', 'bios_contract', [a.bios])
    assert 'BIOS_CONTRACT FCRC cases=36 passed=36' in crc
    pointer = run('pointer', 'bios_io_contract', [a.bios])
    assert 'BIOS_IO_CONTRACT checks=32 passed=32' in pointer
    reads = []
    buffers = []
    for mode, flags in [('Pal', []), ('Ntsc', ['--ntsc'])]:
        text = run('files-' + mode.lower(), 'bios_490_contract', [a.bios, a.dvc, a.disc, *flags])
        assert f'BIOS_490_FILE_CONTRACT mode={mode} checks=42 passed=42' in text
        rows = re.findall(r'FILE mode=' + mode + r' name=(\S+) bytes=(\d+) sha1=([0-9a-f]{40}) PASS', text)
        assert len(rows) == 4
        for name, count, sha in rows:
            passed = hashes.get(name) == (int(count), sha)
            reads.append({'mode': mode, 'name': name, 'bytes': int(count), 'sha1': sha, 'passed': passed})
            assert passed, 'BIOS read differs from independent disc audit'
        print(mode, '42 file checks PASS', flush=True)
        text = run('buffers-' + mode.lower(), 'bios_490_contract', [a.bios, a.dvc, a.disc, *flags, '--buffers'])
        assert f'BIOS_490_BUFFER_CONTRACT mode={mode} PASS' in text
        rows = re.findall(r'BUFFER index=(\d+) ctrl=01 count=2324 pcb=([0-9a-f]{4})', text)
        assert [int(index) for index, _ in rows] == [0, 1]
        buffers.append({'mode': mode, 'buffers': 2, 'payload_bytes_each': 2324, 'passed': True})
        print(mode, 'two linked video buffers PASS', flush=True)
    unchanged = all(digest(Path(f)) == sha for f,sha in identities.items())
    result = {'inputs_sha256': identities, 'inputs_unchanged': unchanged, 'crc_checks': 36,
              'pointer_checks': 32, 'file_checks': 84, 'read_hash_comparisons': reads,
              'linked_video_buffers': buffers,
              'scope': 'Identified 490/IMPEG, one Third Degree fixture, two linked video buffers in each video mode; not all files, continuous streaming or original-hardware timing.'}
    (a.output / 'results.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    assert unchanged
    print('CRC 36 PASS; pointer 32 PASS; input images unchanged', flush=True)


if __name__ == '__main__': main()
