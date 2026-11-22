# Davilize design research brief

Scope: how Tableau, Power BI, Metabase, Superset, Looker Studio, Observable/Evidence and Grafana handle UX, and what Davilize should adopt or avoid.
Evidence key: **[F]** = read from the cited page this session. **[K]** = general product knowledge, not re-verified; lower confidence, verify before relying on it.
Superset, Looker Studio and Observable pages could not be retrieved (empty, redirect, 404), so items about them are [K].

## 1. Layout and information architecture

What the products do
- Tableau: a worksheet is built from shelves (Columns, Rows, Filters, Pages) plus a Marks card for colour/size/shape/text/tooltip; field list on the left; dashboards are a separate tab that takes sheets from a list. [F] https://help.tableau.com/current/pro/desktop/en-us/buildmanual_shelves.htm , https://help.tableau.com/current/pro/desktop/en-us/dashboards_create.htm
- Power BI: Report/Data/Model views on a left rail, Visualizations/Fields panes on the right, and a Selection pane listing every canvas object. [F for Selection pane] https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-accessibility-creating-reports
- Grafana: edit mode opens a sidebar and toolbar; the sidebar can be hidden or docked and the choice persists per browser; a content outline aids navigation. [F] https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/create-dashboard/
- Metabase: dashboards are tabs of cards; settings are per card in edit mode and do not change the source question; fixed-width vs full-width toggle. [F] https://www.metabase.com/docs/latest/dashboards/introduction
- Looker Studio: canvas in the middle, Properties panel (Setup/Style) and data panel on the right; edit/view toggle at top. [K] https://cloud.google.com/looker/docs/studio
- Superset: separate Dataset, Chart (explore view with Data / Customize tabs) and Dashboard areas. [K] https://superset.apache.org/docs/using-superset/creating-your-first-dashboard/

Patterns to adopt
- Three-zone workspace: left data panel, centre canvas, right contextual properties; every side panel collapsible and dockable, state persisted (Grafana).
- Resizable splits with remembered sizes and minimum widths; a per-chart "focus/maximize" (Power BI Focus mode [F]) instead of cramming detail into tiles.
- Properties panel is contextual: it shows settings for whatever is selected (field, mark, chart, tile, dashboard) rather than many always-open panes (Looker Studio, Power BI) [K].
- Compact/comfortable density setting; honour OS text scale; virtualise long field lists.
- A Selection/Layers list for the dashboard (Power BI Selection pane [F]) doubling as z-order, naming, hide and tab-order editor.

Mistakes to avoid
- Tableau's many always-visible cards/shelves overwhelm beginners [K]; reveal Marks/Filters/Pages progressively.
- Power BI's ribbon plus three right-hand panes eats canvas on laptops [K]; avoid a ribbon.
- Hiding essential actions only in right-click menus; mirror them in a visible toolbar and command palette.

## 2. First run, empty states, onboarding, suggestions

What the products do
- Grafana's empty dashboard offers two routes: describe it to an assistant, or add a visualization manually; after a query it proposes visualizations with live preview. [F] https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/create-dashboard/
- Tableau "Show Me": select fields in the data pane and it suggests suitable chart types; you can also double-click fields, drag to shelves, or drop onto the "Drop field here" grid to get a table. [F] https://help.tableau.com/current/pro/desktop/en-us/buildmanual_shelves.htm
- Metabase offers dashboard "sections" (preset layouts such as KPI row + large chart) to fill with questions. [F] https://www.metabase.com/docs/latest/dashboards/introduction
- Evidence: reports are Markdown + SQL with components, local CLI preview, no account needed for Core. Text-first, local-first onboarding. [F] https://docs.evidence.dev/
- Power BI Desktop opens with a start dialog and get-data prompts; Looker Studio starts from templates. [K]

Patterns to adopt
- Empty states are the onboarding: each empty panel says what goes there, offers one primary action ("Open a file", "Drag a field here") and one secondary ("Use sample data").
- Bundled sample dataset plus a pre-built sample report so first run shows a finished result within seconds; clearly labelled and removable.
- "Suggested charts": when fields are selected, show 3-5 ranked chart thumbnails with a one-line reason ("1 date + 1 measure: line"); one click applies and the shelves visibly fill so users learn the mapping (Show Me, Grafana suggestions).
- Drop zones show ghost text ("Drop a measure") and highlight valid targets while dragging; invalid drops explain why.
- Contextual help: tooltips on every icon-only button, a "?" link per panel, inline hints that stay dismissed once used.
- A quiet dismissible checklist (Open data, Build a chart, Add to dashboard, Export) rather than a modal tour.

Mistakes to avoid
- Forced multi-step tours and modal welcome walls.
- Suggestions that silently replace the user's chart; always preview and allow undo.
- Empty canvases with no affordance (blank dashboard grid).

## 3. Core workflow and naming

Observed flows
- Tableau: Connect, Data Source page, Sheet (shelves), Dashboard (drag sheets; "Use as Filter" on a sheet), publish. Terms: workbook, data source, worksheet, dashboard, marks, dimensions/measures, parameters, calculated field. [F] https://help.tableau.com/current/pro/desktop/en-us/dashboards_create.htm
- Metabase: Question (saved chart/query), Dashboard of cards, filter widgets mapped to question fields, then subscriptions/PDF/public link. A question saved into one dashboard is bound to it, so shared charts live in a collection. [F] https://www.metabase.com/docs/latest/dashboards/introduction
- Power BI: Get data, Transform (Power Query), report pages of visuals, slicers, Publish. [K]
- Superset: Database, Dataset, Chart, Dashboard with native filters. [K]
- Grafana: data source, panel, dashboard; variables and URL variables as parameters; library panels for reuse; version history; JSON model. [F] https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/

Patterns to adopt
- One noun per concept, used everywhere: Data (files), Field, Calculated field, Parameter, Chart, Report, Dashboard, Tile, Filter. Avoid synonyms (visual/sheet/question/panel/card).
- Charts are reusable objects; a dashboard tile references one; editing the chart updates all tiles (Grafana library panels, Metabase collections avoid copy drift). [F]
- "Add to dashboard" is available from the chart builder with a dashboard picker and "new dashboard"; each tile has "Edit chart" jumping back.
- Filters show their scope ("This chart / This dashboard / Whole report"). Parameters are named, typed, have defaults, and are usable in calculated fields and text/titles (Metabase variables in text cards [F]).
- Clicking a mark can filter other tiles as an opt-in per-tile toggle (Tableau "Use as Filter" [F]).
- Export/publish is a clear final step: PNG/PDF/CSV per chart and a standalone HTML bundle (Evidence static-site model [F]).
- "View data" on any chart, which also serves accessibility.

Mistakes to avoid
- Tableau drops a filter when you swap its sheet [F]; warn, and keep bindings where possible.
- Forcing users to set dimension/measure roles before anything renders; infer and allow override.
- Different names for the same thing in menus vs docs.

## 4. Dashboard editors

Observed behaviour
- Tableau: tiled vs floating objects; horizontal/vertical containers govern resizing; hiding a tiled object lets neighbours fill the space, hiding a floating one reveals what is beneath. [F] https://help.tableau.com/current/pro/desktop/en-us/dashboards_create.htm
- Metabase: pencil icon enters edit mode; cards snap to a grid and neighbours shift out of the way; add, move, resize, duplicate, replace, remove; in view mode a title click opens the query. [F]
- Grafana: Edit button; "custom" free layout is default, "auto grid" gives a uniform grid with column/row-height settings; hover actions (edit, copy, duplicate, delete); rows/tabs grouping; resizing in auto grid prompts to switch layout. [F] https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/create-dashboard/
- Power BI: gridlines, snap, lock, align/distribute, Selection pane, Ctrl+Z / Ctrl+Y undo. [K]
- Looker Studio: autosave with View/Edit toggle. [K]

Patterns to adopt
- Explicit edit vs view mode with a prominent toggle; view mode hides handles, keeps filters/parameters live, and shows an unsaved-changes indicator.
- 12 or 24 column grid with a row unit; snapping on by default with a modifier to bypass; collisions push neighbours down (Metabase), with an option for free placement.
- Complete keyboard path: Tab focuses a tile; arrows move one cell; Shift+arrows resize; Delete removes; Ctrl+D duplicates; Esc deselects; live-region announcements ("Moved to column 4, row 2"). This satisfies WCAG 2.5.7 (non-drag alternative).
- Undo/redo of whole layout operations (move, resize, add, delete, property change) with Ctrl+Z / Ctrl+Shift+Z and a visible history.
- Minimum tile size; fixed-width canvas (reproducible export) vs fluid (Metabase's fixed/full-width toggle [F]).
- Align/distribute, duplicate, lock and z-order from the layers list; page presets (Letter, A4, 16:9) for export.
- Autosaved local drafts plus version history (Grafana [F]).

Mistakes to avoid
- Mixed tiled/floating models make resize behaviour confusing (Tableau [K]); use grid-only in v1, add floating overlays later.
- Prompting about layout mode on resize (Grafana [F]) is friction; pick one model.
- Silent push-around of other tiles without undo.

## 5. Accessibility

What the products do
- Power BI: all visuals keyboard navigable and screen-reader compatible; authors set alt text (250 chars, can be dynamic), tab order (decorative items removed), titles; "Show data" table via Alt+Shift+F11 in the visual's sort order; Shift+? lists shortcuts; high-contrast themes follow Windows; checklist: avoid tooltip-only information, avoid colour-only encoding (use marker shapes), 4.5:1 text contrast, no autoplay, few visuals per page. [F] https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-accessibility-creating-reports
- Tableau: aims at WCAG 2.2 AA for views; ARIA roles, chart text equivalents, alt text (Cloud 23.2+), colour-blind palette, View Data window. [F] https://help.tableau.com/current/pro/desktop/en-us/accessibility_overview.htm

WCAG 2.2 AA items relevant to dashboards (1.4.11 and 2.5.8 details [F]; other criterion numbers [K])
- 1.1.1 Non-text content: each chart needs a text alternative (short alt text plus data table or description).
- 1.3.1 / 4.1.2: structure and name/role/value for filters, tabs and tiles; tiles as regions with headings.
- 1.4.1 Use of colour: never colour alone; add shapes, patterns or direct labels.
- 1.4.3 Text contrast 4.5:1 (3:1 for large text).
- 1.4.11 Non-text contrast 3:1 for lines, bars, slices, control borders and focus indicators needed to read values; not required when the same information is shown in visible labels. [F] https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html
- 1.4.4 / 1.4.10: text resize to 200% and reflow at 320 CSS px; provide a table fallback where a chart cannot reflow.
- 1.4.13 hover content (chart tooltips) must be dismissible, hoverable and persistent.
- 2.1.1 keyboard operable, 2.1.2 no trap, 2.4.3 focus order, 2.4.7 visible focus, 2.4.11 focus not obscured (sticky toolbars must not cover the focused tile).
- 2.5.7 dragging movements need a single-pointer alternative (grid drag/resize, shelf drag); 2.5.8 targets at least 24x24 CSS px or sufficiently spaced; aim for 44 on key controls. [F for 2.5.8] https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html
- 3.3.x labels and error identification for parameter inputs; 4.1.3 status messages (for example "Filter applied, 124 rows") via live regions.

Patterns to adopt
- Every chart has a title, auto-generated editable summary alt text ("Bar chart of Sales by Region, 5 bars, highest West"), and a one-key "Show as table".
- Data points reachable by keyboard (arrows through marks, value announced, Enter to select/filter).
- Colour-blind-safe default palette with lightness steps between categories; a high-contrast theme; respect prefers-contrast, prefers-reduced-motion and forced-colors.
- Optional marker shapes and dash patterns for line/scatter (Power BI markers [F]).
- Built-in accessibility checker (missing alt text, low contrast, tab order) before export (Power BI checklist [F]).
- Shortcut help dialog on Shift+? (Power BI [F]).
- Exports carry alt text (Power BI to PowerPoint otherwise yields "No alt text provided" [F]); provide CSV beside images.

Mistakes to avoid
- Tooltip-only information and hover-only reveals. [F]
- Colour-only legends; red/green, blue/purple and similar pairs (Power BI lists risky pairs [F]); rainbow scales.
- Animation or autoplay; removing focus outlines; icon-only buttons without accessible names.
- Relying on ARIA on a single canvas element; canvas/SVG charts need a parallel accessible DOM.

## Prioritised recommendations for Davilize (most impactful first)

1. Ship a bundled sample dataset and ready-made sample report so first run produces a finished chart and dashboard within seconds.
2. Use a three-zone layout (data panel, canvas, contextual properties) where every side panel collapses, resizes and remembers its state.
3. Make empty states actionable: each blank panel names its purpose and one primary action, and drop targets show ghost text and highlight valid drops.
4. Add a "Suggested charts" strip that ranks chart types for the selected fields, explains why, previews live, and applies with one undoable click.
5. Fix a single vocabulary (Data, Field, Calculated field, Parameter, Chart, Report, Dashboard, Tile, Filter) and use it in UI, docs and file formats.
6. Store charts as reusable objects that dashboard tiles reference, so edits propagate and "Edit chart" jumps back from any tile.
7. Provide explicit edit vs view modes on dashboards, with handles hidden and filters/parameters live in view mode.
8. Use a snapping 12/24-column grid with push-aside collisions and a non-drag path for every move and resize (keyboard arrows plus numeric position/size fields).
9. Implement undo/redo (Ctrl+Z / Ctrl+Shift+Z) across layout, chart and property changes, with autosaved local drafts and version history.
10. Give every chart auto-generated editable alt text, a one-key "Show as table" view, and keyboard-navigable data points with announced values.
11. Default to a colour-blind-safe palette with 3:1 non-text contrast, optional marker shapes/patterns, and a high-contrast theme honouring forced-colors and reduced-motion.
12. Make filter and parameter scope explicit (chart, dashboard, report), show active filters as chips, and offer click-a-mark-to-filter as a per-tile opt-in.
13. Add an accessibility and layout checker before export that flags missing alt text, low contrast, tab-order problems and undersized tiles.
14. Offer a compact/comfortable density setting, per-chart maximize mode, and a Layers list (name, z-order, hide, tab order) instead of adding toolbars.
15. Make export/publish one final step with PNG/PDF/CSV per chart and a self-contained HTML bundle that carries alt text and data tables.

## Sources
- https://help.tableau.com/current/pro/desktop/en-us/buildmanual_shelves.htm
- https://help.tableau.com/current/pro/desktop/en-us/dashboards_create.htm
- https://help.tableau.com/current/pro/desktop/en-us/accessibility_overview.htm
- https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-accessibility-creating-reports
- https://www.metabase.com/docs/latest/dashboards/introduction
- https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/
- https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/create-dashboard/
- https://docs.evidence.dev/
- https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html
- https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html
- Unverified [K]: https://superset.apache.org/docs/using-superset/creating-your-first-dashboard/ , https://cloud.google.com/looker/docs/studio
