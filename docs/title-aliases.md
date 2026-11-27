# Disc title aliases

The disc-name workbook supplies alternate catalogue names for library category
and missing-title checks, LaunchBox matching, ScreenScraper matching/search and
World of CD-i matching. Original disc paths, save names, artwork filenames and
requested regions remain unchanged. Numbered discs can share a catalogue title
while retaining their individual disc identifiers for media selection.

`assets/title-aliases.json` is the bundled snapshot. The app also reads an optional
`TitleAliases.json` containing additional aliases in its application data folder (`Portable/` beside the executable
for portable installs). This file is read once per app launch. Restart after changing it.
Unknown catalogue slugs are ignored. If an alias identifies more than one catalogue
record, it is rejected rather than selecting the first result.

To apply later spreadsheet additions:

```powershell
python tools/import_title_aliases.py "CD-i disc alternative names.xlsx" "Portable/TitleAliases.json"
```

For the supplied portable copy, run this in the folder containing the workbook:

```powershell
python import_title_aliases.py "CD-i disc alternative names.xlsx" "Portable/TitleAliases.json"
```

Keep `title-aliases-catalogue.json` beside that standalone importer.

The workbook stays editable and the app does not need to parse Excel or contact
the source catalogues at startup. Separate entries with semicolons or line breaks.
Existing source labels are understood by the exporter. Avoid adding explanatory
prose in an alternative-name cell: it is treated as an alias. The NFL Triple Pack
cell is excluded because its user-supplied names are compilation contents, not
equivalent titles.

## Sources and coverage

Prepared 13 November 2026 against the existing 1,355-title World of CD-i snapshot.

* [TOSEC DAT pack, 13 March 2025](https://tosecdev.org/downloads/category/59-2025-03-13):
  CD-i TOSEC-ISO DATs have their own 2012/2013 revisions. 390 matched edition names.
  [TOSEC naming convention](https://www.tosecdev.org/tosec-naming-convention).
* [Official No-Intro CD-i export](https://datomatic.no-intro.org/index.php?page=download&s=164):
  `Non-Redump - Philips - CD-i (20260429-044928)`, 119 exported entries, 17 matched
  edition names. This covers the Non-Redump collection, not every CD-i release.
  [No-Intro naming convention](https://wiki.no-intro.org/index.php?title=Naming_Convention).
* [Redump metadata via libretro](https://github.com/libretro/libretro-database/blob/master/metadat/redump/Philips%20-%20CD-i.dat):
  DAT version 2026.10.07, 1,119 matched edition names. Redump entries are labelled
  Redump, never claimed as official No-Intro entries.
* Twenty existing user aliases are retained as user-supplied names. Their regional
  claims are not represented as independent DAT verification.

469 catalogue titles have source or user aliases. Other rows contain a clearly
labelled title-only convention rendering where useful, or a no-verified-alternate
marker. Title-only renderings do not invent release dates, publishers, regions,
dump flags or disc counts and are not certified full TOSEC/No-Intro filenames.

Only unique title/edition matches and explicitly reviewed spelling corrections
are imported. Fuzzy suggestions are not automatically accepted. Demos, betas and
prototypes remain distinct from retail. Different annual editions, numbered
volumes, sequels, Jr. titles and ambiguous CD-i/VCD editions must not be merged.
This is a title alias index, not hash-based identification of a particular dump.
