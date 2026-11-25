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
| Power BI `.pbix` | yes, model and pages, see below | yes: tables, data snapshot, relationships and calculated columns, see below |

## `.pbix`

A `.pbix` keeps its data model in `DataModel`, an XPress9-compressed Analysis Services
backup. Import decodes it (with the `ripbi-core` crate) and reads the model metadata: tables, calculated columns, DAX
measures, active relationships and the CSV file each table loads. Only the metadata is
read, never the data. Tables loaded from anything other than a CSV file, and DAX
calculated tables, are reported as warnings. If the model cannot be decoded, only the
report pages are imported, with placeholder `<table>.csv` sources and relationships
guessed from CSV files that sit next to the `.pbix`.

### Writing a `.pbix`

`davilize export report.dvz --format pbix --out report.pbix` writes the report's tables
with a **snapshot of their CSV data**, plus the pages and visuals. Power BI Desktop opens
the file, and its own engine returns the same results the CSV files give; refresh it in
Desktop to pick up later changes to the files.

What it stores:

- one table per dataset, with its columns, DAX measures and the Power Query that loads
  the CSV (so Refresh works);
- the data, encoded the way the engine stores it: strings in first-appearance
  dictionaries, whole numbers, decimals and dates in sorted dictionaries, booleans as
  0/1 (single segment, so up to 8,388,608 rows per table); blank cells are real blanks
  (a blank text cell is empty text, shown as "(Blank)"; a blank number, date or boolean is
  the engine's null id);
- relationships, with the hidden index table the engine builds for each, so Power BI
  uses them without any setup. A join on several keys gets a calculated key column on
  both tables (the keys joined with `|`) and one relationship on those columns;
- calculated columns: the DAX expression and its computed values. A `.pbix` holds the
  values, so the exporter needs an engine to evaluate them. The CLI and the app provide
  one; the library function `export` does not, and `export_with` takes an
  `ExportOptions::column_values` callback instead. A report with calculated columns
  exported without one stops with an error.
- datasets that cannot be flattened to a CSV file (custom SQL, or datasets built on
  one) as a snapshot of their rows. The CLI and the app run the dataset in the engine
  and store the result, with the rows also written inline in the table's Power Query
  so the file opens; Power BI cannot refresh such a table. The library needs
  `ExportOptions::dataset_columns` for this and otherwise skips the dataset with a
  warning. Large results make a large query: keep them to some thousands of rows.

Limits:

- Sources that are not CSV files are skipped with a warning, as for the other formats.
- Whole numbers and decimals go through a dictionary rather than the engine's value
  encoding, because the value encoding needs a hidden attribute hierarchy to answer
  `MIN` and `MAX` correctly. The result is the same; the file is somewhat larger.

How it was checked: a throwaway analysis engine (`msmdsrv.exe` from the Desktop install,
sent an `ImageLoad` for the `.pbix` and its `/DataModel` part, as Desktop does) loads the
file and answers DAX queries; the answers were compared with values computed from the CSV
files for every column type, and Desktop itself opened the files and returned the same.
The column file formats are byte-identical to the files Desktop wrote for the same sample
data.

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
- Custom SQL datasets in a workbook or Power BI template (a `.pbix` stores a snapshot),
  row limits, `is_null`-style edge filters in some forms, and
  measure aliases (the output name becomes `<agg>_<field>`).
- Chart types Davilize lacks (pie, area, map, dual axis, combo): imported as the nearest
  chart, with a warning.
- Formatting, colors, hierarchies, quick table calculations and LOD expressions.
- Data paths are absolute in exported files; move the data and the path breaks.
