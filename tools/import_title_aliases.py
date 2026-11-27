"""Export the editable two-column catalogue workbook to TitleAliases.json.

Usage: python tools/import_title_aliases.py workbook.xlsx Portable/TitleAliases.json
Uses only Python's standard library; does not change the workbook or disc files.
"""
import argparse
import json
import re
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path

NS = {'s': 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}
LABELS = ('TOSEC:', 'No-Intro:', 'Redump:', 'Workbook (user):',
          'Title-only format (not a verified DAT entry):')


def export(workbook, catalogue):
    by_name = {}
    for title in catalogue['titles']:
        by_name.setdefault(title['name'], []).append(title['slug'])
    aliases = {}
    excluded = set()
    with zipfile.ZipFile(workbook) as archive:
        strings = []
        if 'xl/sharedStrings.xml' in archive.namelist():
            strings = [''.join(item.itertext()) for item in
                       ET.fromstring(archive.read('xl/sharedStrings.xml'))]
        for path in archive.namelist():
            if not re.fullmatch(r'xl/worksheets/sheet\d+\.xml', path):
                continue
            for row in ET.fromstring(archive.read(path)).findall('.//s:sheetData/s:row', NS):
                cells = {}
                for cell in row.findall('s:c', NS):
                    col = re.sub(r'\d', '', cell.attrib['r'])
                    value = cell.find('s:v', NS)
                    if cell.attrib.get('t') == 's' and value is not None:
                        text = strings[int(value.text)]
                    elif cell.attrib.get('t') == 'inlineStr':
                        text = ''.join(cell.find('s:is', NS).itertext())
                    else:
                        text = value.text if value is not None else ''
                    cells[col] = text
                title, text = cells.get('A', ''), cells.get('B', '')
                ids = by_name.get(title, [])
                if len(ids) != 1:
                    continue
                # The workbook's NFL triple pack cell contains component
                # games, each of which has its own independent catalogue row.
                if title.startswith('NFL') and 'Triple Pack' in title:
                    excluded.add(title)
                    continue
                for line in text.splitlines():
                    line = line.strip()
                    if not line or line.startswith(('No verified alternate', 'Edition requires')):
                        continue
                    for label in LABELS:
                        if line.startswith(label):
                            line = line[len(label):].strip()
                            break
                    for name in line.split(';'):
                        name = name.strip()
                        if name and len(name) <= 4096:
                            aliases.setdefault(ids[0], set()).add(name)
    return {'version': 1, 'titles': [{'slug': slug, 'aliases': sorted(names)}
                                    for slug, names in sorted(aliases.items())]}, excluded


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('workbook', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    bundled = Path(__file__).resolve().parents[1] / 'assets/title-aliases.json'
    if not bundled.is_file():
        bundled = Path(__file__).resolve().parent / 'title-aliases-catalogue.json'
    data, excluded = export(args.workbook, json.loads(bundled.read_text(encoding='utf8')))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding='utf8')
    print(f"Exported {sum(len(t['aliases']) for t in data['titles'])} aliases for {len(data['titles'])} titles.")
    for title in sorted(excluded):
        print(f'Excluded collection contents from automatic aliases: {title}')
