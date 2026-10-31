//! A run's plot as a CSV file, for a spreadsheet or a script.
//!
//! The raw file is the full record. This is the same numbers as text, with the
//! scale the engine leaves unnamed or half saved put back: a DC sweep's scale is
//! called `v-sweep` whatever it varies, and a second sweep's value is not saved at
//! all, so it is rebuilt from the point number.

use super::raw::Plot;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Most values written. The soft-start transient at 0.5 µs is two million and wrote
/// 67 MB in under half a second; past this the text would pass 250 MB, several
/// times the raw file that holds the same data, for every one of the 20 runs kept.
pub(crate) const MAX_CELLS: usize = 10_000_000;

/// How the plot's first variable, its scale, is written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Layout {
    /// The plot's own variables as the raw file names them: a transient's `time`
    /// or an AC sweep's `frequency`.
    Series,
    /// One point, down the rows: an operating point.
    Table,
    /// A DC sweep. The first sweep is the plot's scale, under its own name, and a
    /// second is a column of its own, ahead of it.
    Sweep { inner: String, outer: Option<Outer> },
}

/// The sweep the engine does not save. It steps once each time the first sweep
/// has run, so row `n` sits at step `n / run`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outer {
    pub name: String,
    pub start: f64,
    pub step: f64,
    /// Points in one run of the first sweep.
    pub run: usize,
}

/// Why no file was written, or what was.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Written {
    Bytes(u64),
    TooLarge { cells: usize },
}

fn columns(plot: &Plot) -> usize {
    plot.variables.len() * if plot.complex { 2 } else { 1 }
}

pub(crate) fn cells(plot: &Plot) -> usize {
    plot.points.saturating_mul(columns(plot))
}

/// A header or name as one CSV field. A spreadsheet runs a field that starts with
/// one of `= + - @` as a formula, and a name can come from the design.
fn field(text: &str) -> String {
    let mut field = String::with_capacity(text.len() + 2);
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        field.push('\'');
    }
    field.push_str(text);
    match field.contains([',', '"', '\n', '\r']) {
        true => format!("\"{}\"", field.replace('"', "\"\"")),
        false => field,
    }
}

fn number(value: f64) -> String {
    format!("{value:e}")
}

/// Write `plot` to a new file at `path`. A plot of more than `MAX_CELLS` values is
/// left unwritten.
pub(crate) fn write(path: &Path, plot: &Plot, layout: &Layout) -> std::io::Result<Written> {
    let cells = cells(plot);
    if cells > MAX_CELLS {
        return Ok(Written::TooLarge { cells });
    }
    publish(path, |out| match layout {
        Layout::Table => write_table(out, plot),
        Layout::Series => write_rows(out, plot, None),
        Layout::Sweep { inner, outer } => write_rows(out, plot, Some((inner, outer))),
    })
    .map(Written::Bytes)
}

/// Fill a new file beside `path` and rename it into place, so `path` is either
/// absent or whole. A name that is already taken is refused when it is checked, and
/// the folder is the run's own, so nothing else makes the file in between.
fn publish(
    path: &Path,
    fill: impl FnOnce(&mut BufWriter<std::fs::File>) -> std::io::Result<()>,
) -> std::io::Result<u64> {
    if path.symlink_metadata().is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = std::path::PathBuf::from(partial);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let mut out = BufWriter::new(file);
    let done = fill(&mut out)
        .and_then(|()| out.flush())
        .and_then(|()| std::fs::metadata(&partial))
        .and_then(|meta| std::fs::rename(&partial, path).map(|()| meta.len()));
    drop(out);
    if done.is_err() {
        std::fs::remove_file(&partial).ok();
    }
    done
}

fn write_table(out: &mut impl Write, plot: &Plot) -> std::io::Result<()> {
    writeln!(out, "signal,value")?;
    for (index, variable) in plot.variables.iter().enumerate() {
        let value = plot.real(index).next();
        writeln!(
            out,
            "{},{}",
            field(&variable.name),
            value.map(number).unwrap_or_default()
        )?;
    }
    Ok(())
}

fn write_rows(
    out: &mut impl Write,
    plot: &Plot,
    sweep: Option<(&String, &Option<Outer>)>,
) -> std::io::Result<()> {
    let mut header: Vec<String> = Vec::new();
    if let Some((_, Some(outer))) = sweep {
        header.push(field(&outer.name));
    }
    for (index, variable) in plot.variables.iter().enumerate() {
        match (index, sweep) {
            (0, Some((inner, _))) => header.push(field(inner)),
            // A scale is real even in an AC plot, where its imaginary part is zero.
            (0, None) => header.push(field(&variable.name)),
            _ if plot.complex => {
                header.push(field(&format!("{}.real", variable.name)));
                header.push(field(&format!("{}.imag", variable.name)));
            }
            _ => header.push(field(&variable.name)),
        }
    }
    writeln!(out, "{}", header.join(","))?;

    let mut real: Vec<_> = (0..plot.variables.len())
        .map(|index| plot.real(index))
        .collect();
    let mut imaginary: Vec<_> = (0..plot.variables.len())
        .map(|index| plot.imaginary(index))
        .collect();
    let mut line = String::new();
    for row in 0..plot.points {
        line.clear();
        if let Some((_, Some(outer))) = sweep {
            let step = row / outer.run.max(1);
            line.push_str(&number(outer.start + outer.step * step as f64));
            line.push(',');
        }
        for index in 0..real.len() {
            if index > 0 {
                line.push(',');
            }
            // A variable shorter than the plot leaves its cells empty.
            let (re, im) = (real[index].next(), imaginary[index].next());
            if let Some(value) = re {
                line.push_str(&number(value));
            }
            if plot.complex && index > 0 {
                line.push(',');
                if let Some(value) = im {
                    line.push_str(&number(value));
                }
            }
        }
        line.push('\n');
        out.write_all(line.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::raw;

    fn plot(text: &str) -> Plot {
        raw::parse(text.as_bytes()).unwrap().pop().unwrap()
    }

    fn real_plot(names: &[&str], rows: &[&[f64]]) -> Plot {
        let mut text = format!(
            "Title: t\nPlotname: p\nFlags: real\nNo. Variables: {}\nNo. Points: {}\nVariables:\n",
            names.len(),
            rows.len()
        );
        for (index, name) in names.iter().enumerate() {
            text.push_str(&format!("\t{index}\t{name}\tvoltage\n"));
        }
        text.push_str("Values:\n");
        for (index, row) in rows.iter().enumerate() {
            for (column, value) in row.iter().enumerate() {
                let lead = if column == 0 {
                    format!("{index}")
                } else {
                    String::new()
                };
                text.push_str(&format!("{lead}\t{value:e}\n"));
            }
        }
        text.push('\n');
        plot(&text)
    }

    fn written(plot: &Plot, layout: &Layout) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        assert!(matches!(
            write(&path, plot, layout).unwrap(),
            Written::Bytes(_)
        ));
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_transient_keeps_the_raw_names_and_values() {
        let plot = real_plot(&["time", "v(a)"], &[&[0.0, 1.5], &[1e-3, 2.5]]);
        assert_eq!(
            written(&plot, &Layout::Series),
            "time,v(a)\n0e0,1.5e0\n1e-3,2.5e0\n"
        );
    }

    #[test]
    fn a_complex_sweep_has_a_real_scale_and_two_columns_a_signal() {
        let plot = plot(
            "Title: t\nPlotname: AC Analysis\nFlags: complex\nNo. Variables: 2\nNo. Points: 2\n\
             Variables:\n\t0\tfrequency\tfrequency grid=3\n\t1\tv(a)\tvoltage\nValues:\n\
             0\t1e0,0\n\t0.5,-0.25\n1\t1e1,0\n\t0.25,-0.5\n\n",
        );
        assert_eq!(
            written(&plot, &Layout::Series),
            "frequency,v(a).real,v(a).imag\n1e0,5e-1,-2.5e-1\n1e1,2.5e-1,-5e-1\n"
        );
    }

    #[test]
    fn a_dc_sweep_is_named_for_what_it_varies_and_not_for_the_engines_scale() {
        let plot = real_plot(&["v-sweep", "v(a)"], &[&[0.0, 1.0], &[0.5, 2.0]]);
        let layout = Layout::Sweep {
            inner: "vin (V)".to_string(),
            outer: None,
        };
        assert_eq!(written(&plot, &layout), "vin (V),v(a)\n0e0,1e0\n5e-1,2e0\n");
    }

    #[test]
    fn the_second_sweep_is_rebuilt_from_the_point_number() {
        // Two runs of three points: the first sweep goes 0, 1, 2 twice.
        let plot = real_plot(
            &["v-sweep", "v(a)"],
            &[
                &[0.0, 10.0],
                &[1.0, 11.0],
                &[2.0, 12.0],
                &[0.0, 20.0],
                &[1.0, 21.0],
                &[2.0, 22.0],
            ],
        );
        let layout = Layout::Sweep {
            inner: "vin (V)".to_string(),
            outer: Some(Outer {
                name: "temp (°C)".to_string(),
                start: -40.0,
                step: 125.0,
                run: 3,
            }),
        };
        let text = written(&plot, &layout);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "temp (°C),vin (V),v(a)");
        assert_eq!(lines[1], "-4e1,0e0,1e1");
        assert_eq!(lines[3], "-4e1,2e0,1.2e1");
        assert_eq!(lines[4], "8.5e1,0e0,2e1");
        assert_eq!(lines[6], "8.5e1,2e0,2.2e1");
    }

    #[test]
    fn an_operating_point_reads_down_the_rows() {
        let plot = real_plot(&["v(a)", "i(v1)"], &[&[5.0, -0.5]]);
        assert_eq!(
            written(&plot, &Layout::Table),
            "signal,value\nv(a),5e0\ni(v1),-5e-1\n"
        );
    }

    #[test]
    fn a_name_cannot_run_as_a_formula_or_split_a_row() {
        assert_eq!(field("=1+1"), "'=1+1");
        assert_eq!(field("-x"), "'-x");
        assert_eq!(field("@sum"), "'@sum");
        assert_eq!(field("+1"), "'+1");
        assert_eq!(field("\tcell"), "'\tcell");
        assert_eq!(field("\rcell"), "\"'\rcell\"");
        assert_eq!(field("v(a,b)"), "\"v(a,b)\"");
        assert_eq!(field("a\"b"), "\"a\"\"b\"");
        assert_eq!(field("v(/out)"), "v(/out)");
        let plot = real_plot(&["time", "=cmd(a,b)"], &[&[0.0, 1.0]]);
        assert_eq!(
            written(&plot, &Layout::Series),
            "time,\"'=cmd(a,b)\"\n0e0,1e0\n"
        );
    }

    #[test]
    fn a_plot_past_the_limit_writes_no_file() {
        let plot = real_plot(&["time", "v(a)"], &[&[0.0, 1.0]]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let mut big = plot.clone();
        big.points = MAX_CELLS;
        assert_eq!(
            write(&path, &big, &Layout::Series).unwrap(),
            Written::TooLarge {
                cells: MAX_CELLS * 2
            }
        );
        assert!(!path.exists());
    }

    #[test]
    fn a_file_that_is_there_is_never_overwritten() {
        let plot = real_plot(&["time", "v(a)"], &[&[0.0, 1.0]]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        std::fs::write(&path, "mine").unwrap();
        assert!(write(&path, &plot, &Layout::Series).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
    }

    #[test]
    fn a_write_that_fails_leaves_neither_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let failed = publish(&path, |out| {
            out.write_all(b"half a row")?;
            Err(std::io::Error::other("disk full"))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let written = publish(&path, |out| out.write_all(b"whole")).unwrap();
        assert_eq!(written, 5);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "whole");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
