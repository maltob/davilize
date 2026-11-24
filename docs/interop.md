# Tableau and Power BI import and export

Davilize converts reports to and from Tableau and Power BI files. Conversion is
best effort: whatever the Davilize model cannot express is reported as a warning
(on stderr in the CLI, in the toast in the app), and the rest still converts.

**Not yet checked against the real applications.** The writers follow the file
formats as documented and as seen in sample files, and the tests round-trip every
format through the importers and the engine. Nobody has opened the output in Tableau
Desktop or Power BI Desktop yet. If one refuses a file, please report it with the
application's error message.

## Commands

```powershell
davilize import <file|folder> --out report.dvz
davilize export report.dvz --format twb|twbx|pbit|pbip --out <path>
```

In the app, **Import...** (next to **Open report**) converts a file, saves the
report next to it (`<name>.dvz`, or `<name> (2).dvz` if that exists) and opens it.
The export list has the four formats below. A Power BI Project folder is imported
with the CLI.

| Format | Import | Export |
|---|---|---|
| Tableau `.twb` | yes | yes |
| Tableau `.twbx` | yes (CSV files inside are unpacked to `<name>_data/`) | yes (CSV files packed under `Data/`) |
| Power BI `.pbit` template | yes | yes |
| Power BI Project folder (TMDL + PBIR, `.pbip`) | yes | yes |
| Power BI `.pbix` | pages only, see below | no |

## `.pbix`

A `.pbix` keeps its data model in a compressed binary that cannot be read. Importing
one reads the report pages only; each table the visuals use becomes a placeholder CSV
source (`<table>.csv`) that you must point at your data. To produce a `.pbix`, export a
`.pbit` or a project folder, open it in Power BI Desktop and save as `.pbix`.

## What converts

| Davilize | Tableau | Power BI |
|---|---|---|
| CSV source | text file connection | CSV query (`Csv.Document`) |
| Dataset | datasource | table |
| Join (equality keys) | join relation | relationship (one key; first kept) |
| Calculated field | calculation (row level) | calculated column (DAX) |
| Parameter | parameter | not exported; defaults replace it in filters |
| Bar, line, scatter, table | same marks | clustered column/bar, line, scatter, table |
| Heatmap | square mark | matrix (add conditional formatting) |
| Filters, sort | filters, sort | filters, sort |
| Dashboard | dashboard (zones) | page |

Dashboard layouts are scaled between Davilize's grid and the other tool's coordinates.
Shared dashboard filters are applied to each visual in Power BI and are not exported
to Tableau.

Calculated fields are translated between DuckDB, Tableau and DAX for common
row-level expressions (`IF`/`CASE`, text, math, date and null functions). Anything with
aggregates, level-of-detail expressions, table calculations, `CALCULATE`, clock
functions or parameter references is skipped with a warning, except a calculation that
is exactly one aggregate (`SUM([Sales])`, `SUM(Sales[Amount])`), which becomes a measure
on the visuals that use it. Imported expressions are checked against the calculated-field
allowlist and dropped if refused.

## What does not convert

- Sources other than CSV when exporting (databases, Parquet, JSON, Arrow). Databases
  and extracts (`.hyper`) are skipped when importing.
- Custom SQL datasets, row limits, `is_null`-style edge filters in some forms, and
  measure aliases (the output name becomes `<agg>_<field>`).
- Chart types Davilize lacks (pie, area, map, dual axis, combo): imported as the nearest
  chart, with a warning.
- Formatting, colors, hierarchies, quick table calculations and LOD expressions.
- Data paths are absolute in exported files; move the data and the path breaks.
