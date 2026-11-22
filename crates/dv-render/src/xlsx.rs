//! XLSX workbook with one sheet per visual. Cells keep their types: numbers
//! are numeric, booleans are booleans, dates are Excel date-times.

use rust_xlsxwriter::{Format, Workbook, XlsxError};

use crate::table::Cell;
use crate::{RenderError, Rendered};

/// Excel limits sheet names to 31 characters.
const MAX_SHEET_NAME: usize = 31;

/// Builds the workbook in memory and returns its bytes.
pub fn workbook(visuals: &[Rendered]) -> Result<Vec<u8>, RenderError> {
    let mut book = Workbook::new();
    let header = Format::new().set_bold();
    let datetime = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");

    if visuals.is_empty() {
        let sheet = book.add_worksheet();
        sheet.set_name("empty").map_err(xlsx_error)?;
    }
    for visual in visuals {
        let sheet = book.add_worksheet();
        let name: String = visual.id.chars().take(MAX_SHEET_NAME).collect();
        sheet.set_name(name).map_err(xlsx_error)?;

        for (col, column) in visual.table.columns.iter().enumerate() {
            sheet
                .write_string_with_format(0, col as u16, &column.name, &header)
                .map_err(xlsx_error)?;
        }
        for (row, cells) in visual.table.rows.iter().enumerate() {
            let row = (row + 1) as u32;
            for (col, cell) in cells.iter().enumerate() {
                let col = col as u16;
                match cell {
                    Cell::Null => {}
                    Cell::Number(n) => {
                        sheet.write_number(row, col, *n).map_err(xlsx_error)?;
                    }
                    Cell::Bool(b) => {
                        sheet.write_boolean(row, col, *b).map_err(xlsx_error)?;
                    }
                    Cell::Text(text) => {
                        sheet.write_string(row, col, text).map_err(xlsx_error)?;
                    }
                    Cell::Time { serial, .. } => {
                        sheet
                            .write_number_with_format(row, col, *serial, &datetime)
                            .map_err(xlsx_error)?;
                    }
                }
            }
        }
        sheet.set_freeze_panes(1, 0).map_err(xlsx_error)?;
        sheet.set_autofit_max_width(40).autofit();
    }
    book.save_to_buffer().map_err(xlsx_error)
}

fn xlsx_error(err: XlsxError) -> RenderError {
    RenderError::Output(format!("cannot write workbook: {err}"))
}
