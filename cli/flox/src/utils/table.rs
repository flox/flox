use std::io::Write;

use tabwriter::TabWriter;

/// Aligns `\t`-separated cells into columns for tabular output.
///
/// ANSI escapes don't count towards a column's width, so styled cells stay
/// aligned.
pub fn table_writer<W: Write>(w: W) -> TabWriter<W> {
    TabWriter::new(w).ansi(true)
}

/// Returns the aligned table written to a [table_writer].
pub fn table_string(tw: TabWriter<Vec<u8>>) -> String {
    // Flushing into a `Vec` can't fail, and `TabWriter` only adds spaces to
    // the UTF-8 that `write!` produces.
    let table = tw.into_inner().expect("writing to a Vec can't fail");
    String::from_utf8(table).expect("TabWriter output is UTF-8")
}
