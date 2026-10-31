//! Read the result files ngspice writes.
//!
//! The format is what `raw_write` in ngspice's `rawfile.c` produces: header lines,
//! a `Variables:` list, then every value of every variable, point by point, as
//! text after `Values:` or as native doubles after `Binary:`. A complex plot holds
//! two numbers per value, and its scale is written as complex too.
//!
//! A file can end early when the engine dies while writing it. That is reported
//! as fewer points than the header declared, keeping every whole point before the
//! cut, so a damaged run still gives what it has and never reads as complete.
//!
//! This reads what `write` produces. A halted run writes a whole file with fewer
//! points, and nothing in the file says so: only the worker knows.
use std::path::Path;

const DOUBLE_BYTES: usize = std::mem::size_of::<f64>();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    /// ngspice's type name, such as `voltage`, `current` or `time`.
    pub kind: String,
    /// Its own length when that differs from the plot, from `dims=`. The file
    /// pads a shorter variable with zeros, which are not data.
    pub length: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub title: String,
    pub name: String,
    pub complex: bool,
    /// The `Dimensions:` line, if any. A nested `.dc` written by `write` has none.
    pub dimensions: Vec<usize>,
    pub variables: Vec<Variable>,
    pub declared_points: usize,
    pub points: usize,
    /// Point by point, with a real and imaginary part per value if complex.
    values: Vec<f64>,
}

impl Plot {
    /// Fewer whole points in the file than its header declared.
    pub fn is_truncated(&self) -> bool {
        self.points < self.declared_points
    }
    /// The one variable with this name, ignoring case because ngspice stores
    /// names in lowercase. Two can share a name, as a `dac_bridge` with two ports
    /// does, and picking one would report the wrong current.
    pub fn variable(&self, name: &str) -> Option<usize> {
        let mut matches = self
            .variables
            .iter()
            .enumerate()
            .filter(|(_, variable)| variable.name.eq_ignore_ascii_case(name))
            .map(|(index, _)| index);
        let first = matches.next();
        first.filter(|_| matches.next().is_none())
    }
    fn numbers_per_value(&self) -> usize {
        if self.complex {
            2
        } else {
            1
        }
    }

    /// One variable's real parts, point by point.
    pub fn real(&self, variable: usize) -> impl Iterator<Item = f64> + '_ {
        self.column(variable, 0)
    }

    /// One variable's imaginary parts, which are all zero in a real plot.
    pub fn imaginary(&self, variable: usize) -> impl Iterator<Item = f64> + '_ {
        let part = usize::from(self.complex);
        self.column(variable, part)
            .map(|value| if self.complex { value } else { 0.0 })
    }

    fn column(&self, variable: usize, part: usize) -> impl Iterator<Item = f64> + '_ {
        let width = self.numbers_per_value();
        let row = self.variables.len() * width;
        let start = variable * width + part;
        let own_length = self.variables.get(variable).map_or(0, |variable| {
            variable
                .length
                .map_or(self.points, |length| length.min(self.points))
        });
        self.values
            .iter()
            .skip(start)
            .step_by(row)
            .take(own_length)
            .copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RawError {
    #[error("cannot read the raw file: {0}")]
    Unreadable(String),
    #[error("the raw file is too large, at {0} bytes")]
    TooLarge(u64),
    #[error("this is not an ngspice raw file")]
    NotARawFile,
    #[error("the raw file ends inside its header")]
    HeaderCutShort,
    #[error("bad raw file header on line {line}: {detail}")]
    BadHeader { line: usize, detail: &'static str },
    /// The engine fell back to its constants plot, because no analysis left data.
    #[error("the raw file holds no results")]
    NoResults,
    /// Written with `nopadding`, whose rows have no fixed width.
    #[error("unpadded raw files are not supported")]
    Unpadded,
    #[error("bad value on raw file line {line}")]
    BadValue { line: usize },
}

pub fn read(path: &Path, max_bytes: u64) -> Result<Vec<Plot>, RawError> {
    let unreadable = |error: std::io::Error| RawError::Unreadable(error.to_string());
    let size = std::fs::metadata(path).map_err(unreadable)?.len();
    if size > max_bytes {
        return Err(RawError::TooLarge(size));
    }
    parse(&std::fs::read(path).map_err(unreadable)?)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Plot>, RawError> {
    let mut reader = Reader {
        bytes,
        at: 0,
        line: 0,
    };
    let mut plots = Vec::new();
    while reader.skip_blank_lines() {
        let header = reader.header()?;
        let (values, points) = match header.encoding {
            Encoding::Text => reader.text_values(&header)?,
            Encoding::Binary => reader.binary_values(&header)?,
        };
        plots.push(Plot {
            title: header.title,
            name: header.name,
            complex: header.complex,
            dimensions: header.dimensions,
            variables: header.variables,
            declared_points: header.points,
            points,
            values,
        });
        // A short plot means the file was cut, so nothing can follow it.
        if plots.last().is_some_and(Plot::is_truncated) {
            break;
        }
    }
    if plots.is_empty() {
        return Err(RawError::NotARawFile);
    }
    let holds_results = |plot: &Plot| !plot.name.eq_ignore_ascii_case("constants");
    if !plots.iter().any(holds_results) {
        return Err(RawError::NoResults);
    }
    Ok(plots)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Text,
    Binary,
}

struct Header {
    title: String,
    name: String,
    complex: bool,
    dimensions: Vec<usize>,
    variables: Vec<Variable>,
    points: usize,
    encoding: Encoding,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    line: usize,
}

/// `key:` at the start of a line, ignoring case, as ngspice reads it.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let head = line.get(..key.len())?;
    head.eq_ignore_ascii_case(key)
        .then(|| line[key.len()..].trim())
}

impl<'a> Reader<'a> {
    /// The next line, and whether a newline ended it. A cut file can stop part
    /// way through a number, so only a finished line can be trusted.
    fn next_line(&mut self) -> Option<(&'a [u8], bool)> {
        let bytes = self.bytes;
        let rest = bytes.get(self.at..).filter(|rest| !rest.is_empty())?;
        self.line += 1;
        let end = rest.iter().position(|byte| *byte == b'\n');
        self.at += end.map_or(rest.len(), |end| end + 1);
        let line = &rest[..end.unwrap_or(rest.len())];
        Some((line, end.is_some()))
    }

    /// False at the end of the file.
    fn skip_blank_lines(&mut self) -> bool {
        loop {
            let (start, line) = (self.at, self.line);
            match self.next_line() {
                None => return false,
                Some((text, _)) if text.iter().all(u8::is_ascii_whitespace) => {}
                Some(_) => {
                    self.at = start;
                    self.line = line;
                    return true;
                }
            }
        }
    }

    fn bad_header(&self, detail: &'static str) -> RawError {
        RawError::BadHeader {
            line: self.line,
            detail,
        }
    }

    fn count(&self, text: &str) -> Result<usize, RawError> {
        text.parse()
            .map_err(|_| self.bad_header("this count is not a number"))
    }

    fn header(&mut self) -> Result<Header, RawError> {
        let mut title = None;
        let mut name = String::new();
        let mut complex = None;
        let mut dimensions = Vec::new();
        let mut variable_count = None;
        let mut points = None;
        let mut variables = Vec::new();
        loop {
            let Some((raw, _)) = self.next_line() else {
                return Err(if title.is_some() {
                    RawError::HeaderCutShort
                } else {
                    RawError::NotARawFile
                });
            };
            let text = String::from_utf8_lossy(raw);
            let line = text.as_ref();
            let data_starts = field(line, "values:").is_some() || field(line, "binary:").is_some();
            if !variables.is_empty() && !data_starts {
                return Err(self.bad_header("the data must follow the variables"));
            }
            if let Some(value) = field(line, "title:") {
                title = Some(value.to_string());
            } else if title.is_none() {
                return Err(RawError::NotARawFile);
            } else if let Some(value) = field(line, "plotname:") {
                name = value.to_string();
            } else if let Some(value) = field(line, "flags:") {
                complex = Some(self.flags(value)?);
            } else if let Some(value) = field(line, "no. variables:") {
                variable_count = Some(self.count(value)?);
            } else if let Some(value) = field(line, "no. points:") {
                points = Some(self.count(value)?);
            } else if let Some(value) = field(line, "dimensions:") {
                dimensions = value
                    .split(',')
                    .map(|size| self.count(size.trim()))
                    .collect::<Result<_, _>>()?;
            } else if field(line, "variables:").is_some() {
                variables = self.variables(variable_count.unwrap_or(0))?;
            } else if field(line, "values:").is_some() || field(line, "binary:").is_some() {
                let encoding = if field(line, "binary:").is_some() {
                    Encoding::Binary
                } else {
                    Encoding::Text
                };
                if variables.is_empty() {
                    return Err(self.bad_header("the data comes before any variables"));
                }
                return Ok(Header {
                    title: title.unwrap_or_default(),
                    name,
                    complex: complex.ok_or_else(|| self.bad_header("the header has no flags"))?,
                    dimensions,
                    variables,
                    points: points
                        .ok_or_else(|| self.bad_header("the header has no point count"))?,
                    encoding,
                });
            }
        }
    }

    fn flags(&self, value: &str) -> Result<bool, RawError> {
        let mut complex = None;
        for flag in value.split_whitespace() {
            match flag.to_ascii_lowercase().as_str() {
                "real" => complex = Some(false),
                "complex" => complex = Some(true),
                "unpadded" => return Err(RawError::Unpadded),
                _ => {}
            }
        }
        complex.ok_or_else(|| self.bad_header("the flags say neither real nor complex"))
    }

    /// `\t0\tv(/out)\tvoltage grid=3`, one line per variable.
    fn variables(&mut self, count: usize) -> Result<Vec<Variable>, RawError> {
        let mut variables = Vec::new();
        for index in 0..count {
            let (raw, _) = self.next_line().ok_or(RawError::HeaderCutShort)?;
            let text = String::from_utf8_lossy(raw);
            let mut fields = text.split('\t').filter(|field| !field.is_empty());
            let number = fields.next().map(|field| field.trim().parse::<usize>());
            let name = fields.next();
            let details = fields.next();
            let kind = details.and_then(|field| field.split_whitespace().next());
            match (number, name, kind) {
                (Some(Ok(number)), Some(name), Some(kind)) if number == index => {
                    variables.push(Variable {
                        name: name.to_string(),
                        kind: kind.to_string(),
                        length: details.and_then(own_length),
                    });
                }
                _ => return Err(self.bad_header("this line is not the next variable")),
            }
        }
        Ok(variables)
    }

    /// Every whole point, and how many there were. Only a finished line counts,
    /// and a line that is not a number stops the plot if the file ends there.
    fn text_values(&mut self, header: &Header) -> Result<(Vec<f64>, usize), RawError> {
        let variables = header.variables.len();
        let mut values = Vec::new();
        for point in 0..header.points {
            let row_start = values.len();
            // ngspice leaves a blank line after each point.
            self.skip_blank_lines();
            for variable in 0..variables {
                let line = self.next_line().filter(|(_, finished)| *finished);
                let Some((raw, _)) = line else {
                    values.truncate(row_start);
                    return Ok((values, point));
                };
                let parsed = text_value(raw, variable == 0, point, header.complex);
                let Some((real, imaginary)) = parsed else {
                    return Err(RawError::BadValue { line: self.line });
                };
                values.push(real);
                values.extend(imaginary);
            }
        }
        Ok((values, header.points))
    }

    fn binary_values(&mut self, header: &Header) -> Result<(Vec<f64>, usize), RawError> {
        let width = if header.complex { 2 } else { 1 };
        let row_bytes = header.variables.len() * width * DOUBLE_BYTES;
        let rest = &self.bytes[self.at.min(self.bytes.len())..];
        let points = header.points.min(rest.len() / row_bytes);
        let data = &rest[..points * row_bytes];
        self.at += data.len();
        let values = data
            .chunks_exact(DOUBLE_BYTES)
            .map(|bytes| {
                let mut number = [0; DOUBLE_BYTES];
                number.copy_from_slice(bytes);
                f64::from_ne_bytes(number)
            })
            .collect();
        Ok((values, points))
    }
}

/// The `dims=` attribute, which ngspice writes only when a variable differs from
/// the plot. The product of `dims=3,2` is its length.
fn own_length(details: &str) -> Option<usize> {
    let dims = details
        .split_whitespace()
        .find_map(|attribute| attribute.strip_prefix("dims="))?;
    dims.split(',')
        .map(|size| size.parse::<usize>().ok())
        .product::<Option<usize>>()
}

/// ` 3\t1.5e+00` for a point's first value, `\t1.5e+00` for the rest, and
/// `re,im` in a complex plot.
fn text_value(raw: &[u8], first: bool, point: usize, complex: bool) -> Option<(f64, Option<f64>)> {
    let line = std::str::from_utf8(raw).ok()?;
    let (start, number) = line.split_once('\t')?;
    let index_matches = if first {
        start.trim().parse::<usize>().ok() == Some(point)
    } else {
        start.trim().is_empty()
    };
    if !index_matches {
        return None;
    }
    if complex {
        let (real, imaginary) = number.split_once(',')?;
        Some((
            real.trim().parse().ok()?,
            Some(imaginary.trim().parse().ok()?),
        ))
    } else {
        Some((number.trim().parse().ok()?, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    macro_rules! fixture {
        ($name:literal) => {
            (
                $name,
                include_bytes!(concat!(
                    "../../tests/fixtures/simulation/raw/",
                    $name,
                    ".ascii.raw"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "../../tests/fixtures/simulation/raw/",
                    $name,
                    ".binary.raw"
                ))
                .as_slice(),
                include_str!(concat!(
                    "../../tests/fixtures/simulation/raw/",
                    $name,
                    ".expected.json"
                )),
            )
        };
    }

    fn fixtures() -> [(&'static str, &'static [u8], &'static [u8], &'static str); 6] {
        [
            fixture!("op"),
            fixture!("tran"),
            fixture!("ac"),
            fixture!("dc"),
            fixture!("dc_nested"),
            fixture!("halted"),
        ]
    }

    const OP_TEXT: &[u8] = include_bytes!("../../tests/fixtures/simulation/raw/op.ascii.raw");
    const OP_BINARY: &[u8] = include_bytes!("../../tests/fixtures/simulation/raw/op.binary.raw");
    const AC_TEXT: &[u8] = include_bytes!("../../tests/fixtures/simulation/raw/ac.ascii.raw");
    const AC_BINARY: &[u8] = include_bytes!("../../tests/fixtures/simulation/raw/ac.binary.raw");
    const TRAN_BINARY: &[u8] =
        include_bytes!("../../tests/fixtures/simulation/raw/tran.binary.raw");

    fn only(bytes: &[u8]) -> Plot {
        let mut plots = parse(bytes).unwrap();
        assert_eq!(plots.len(), 1);
        plots.remove(0)
    }

    /// The name ngGet_Vec_Info gives the vector a raw variable holds.
    fn vector_name(variable: &Variable) -> String {
        let inner = |text: &str, head: &str| {
            text.strip_prefix(head)
                .and_then(|rest| rest.strip_suffix(')'))
                .map(str::to_owned)
        };
        if let Some(name) = inner(&variable.name, "i(") {
            return if name.starts_with('@') {
                name
            } else {
                format!("{name}#branch")
            };
        }
        inner(&variable.name, "v(").unwrap_or_else(|| variable.name.clone())
    }

    fn assert_close(actual: f64, expected: f64, context: &str) {
        let scale = expected.abs().max(1e-30);
        assert!(
            (actual - expected).abs() <= 1e-13 * scale,
            "{context}: {actual} against {expected}"
        );
    }

    #[test]
    fn every_real_file_matches_what_the_engine_held_in_memory() {
        for (name, text, binary, expected) in fixtures() {
            let expected: Value = serde_json::from_str(expected).unwrap();
            let vectors = expected["vectors"].as_object().unwrap();
            for bytes in [text, binary] {
                let plot = only(bytes);
                assert!(!plot.is_truncated(), "{name}");
                assert_eq!(plot.variables.len(), vectors.len(), "{name}");
                for (index, variable) in plot.variables.iter().enumerate() {
                    let key = vector_name(variable);
                    let values = vectors[&key].as_array().unwrap_or_else(|| panic!("{key}"));
                    assert_eq!(values.len(), plot.points, "{name} {key}");
                    let real: Vec<f64> = plot.real(index).collect();
                    let imaginary: Vec<f64> = plot.imaginary(index).collect();
                    assert_eq!(imaginary.len(), plot.points);
                    for (at, value) in values.iter().enumerate() {
                        let (re, im) = match value.as_array() {
                            Some(pair) => (pair[0].as_f64().unwrap(), pair[1].as_f64().unwrap()),
                            None => (value.as_f64().unwrap(), 0.0),
                        };
                        assert_close(real[at], re, &format!("{name} {key} {at}"));
                        assert_close(imaginary[at], im, &format!("{name} {key} {at}"));
                    }
                }
            }
        }
    }

    #[test]
    fn text_and_binary_of_one_run_agree() {
        for (name, text, binary, _) in fixtures() {
            let (text, binary) = (only(text), only(binary));
            assert_eq!(text.variables, binary.variables, "{name}");
            assert_eq!(text.points, binary.points, "{name}");
            assert_eq!(text.complex, binary.complex, "{name}");
            assert_eq!(text.values.len(), binary.values.len(), "{name}");
        }
    }

    #[test]
    fn a_header_is_read_in_full() {
        let plot = only(OP_TEXT);
        assert_eq!(plot.title, "konnect simulation");
        assert_eq!(plot.name, "Operating Point");
        assert!(!plot.complex);
        assert_eq!(plot.points, 1);
        assert_eq!(plot.declared_points, 1);
        assert_eq!(plot.variables[1].name, "v(/out)");
        assert_eq!(plot.variables[1].kind, "voltage");
        assert_eq!(plot.variables[2].kind, "current");
    }

    #[test]
    fn known_values_come_out_of_a_real_divider() {
        for bytes in [OP_TEXT, OP_BINARY] {
            let plot = only(bytes);
            let out = plot.variable("V(/OUT)").unwrap();
            assert_eq!(plot.real(out).collect::<Vec<_>>(), [2.0]);
            let source = plot.variable("i(v1)").unwrap();
            assert_close(plot.real(source).next().unwrap(), -1e-3, "supply");
        }
    }

    #[test]
    fn a_complex_plot_keeps_both_parts_and_its_scale_is_complex() {
        for bytes in [AC_TEXT, AC_BINARY] {
            let plot = only(bytes);
            assert!(plot.complex);
            assert_eq!(plot.variables[0].kind, "frequency");
            assert_close(plot.real(0).next().unwrap(), 10.0, "first frequency");
            let out = plot.variable("v(/out)").unwrap();
            let last_re = plot.real(out).last().unwrap();
            let last_im = plot.imaginary(out).last().unwrap();
            assert!(last_im < 0.0 && last_re.hypot(last_im) < 0.1);
        }
    }
    #[test]
    fn a_missing_variable_is_none() {
        assert_eq!(only(OP_TEXT).variable("v(/nope)"), None);
    }

    #[test]
    fn two_variables_with_one_name_are_not_guessed_between() {
        let text = with("\t1\tv(/out)\tvoltage", "\t1\tv(/in)\tvoltage");
        let plot = only(&text);
        assert_eq!(plot.variable("v(/in)"), None);
        assert_eq!(plot.variable("i(v1)"), Some(2));
    }

    #[test]
    fn a_variable_shorter_than_its_plot_stops_at_its_own_length() {
        let bytes = include_bytes!("../../tests/fixtures/simulation/raw/dc.ascii.raw");
        let text =
            String::from_utf8_lossy(bytes).replace("v(/out)\tvoltage", "v(/out)\tvoltage dims=2");
        let plot = only(text.as_bytes());
        let out = plot.variable("v(/out)").unwrap();
        assert_eq!(plot.real(out).count(), 2);
        assert_eq!(plot.imaginary(out).count(), 2);
        assert_eq!(plot.real(0).count(), plot.points);
    }

    #[test]
    fn a_length_of_several_dimensions_is_their_product() {
        assert_eq!(own_length("voltage dims=3,2"), Some(6));
        assert_eq!(own_length("voltage dims=3,x"), None);
        assert_eq!(own_length("voltage grid=3"), None);
    }

    #[test]
    fn a_variable_past_the_end_has_no_values() {
        let plot = only(OP_TEXT);
        assert_eq!(plot.real(plot.variables.len()).count(), 0);
    }

    #[test]
    fn a_constants_plot_is_not_a_result() {
        let text = with("Plotname: Operating Point", "Plotname: constants");
        assert_eq!(parse(&text), Err(RawError::NoResults));
        let text = with("Plotname: Operating Point", "Plotname: Constants");
        assert_eq!(parse(&text), Err(RawError::NoResults));
    }

    #[test]
    fn a_variable_line_past_the_declared_count_is_refused() {
        let text = with("No. Variables: 3", "No. Variables: 2");
        assert!(matches!(parse(&text), Err(RawError::BadHeader { .. })));
    }
    #[test]
    fn cutting_a_file_anywhere_never_panics_or_invents_points() {
        for (name, text, binary, _) in fixtures() {
            for bytes in [text, binary] {
                let whole = only(bytes);
                for cut in 0..bytes.len() {
                    let Ok(plots) = parse(&bytes[..cut]) else {
                        continue;
                    };
                    let plot = &plots[0];
                    assert_eq!(plots.len(), 1, "{name} at {cut}");
                    assert!(plot.points <= whole.points, "{name} at {cut}");
                    assert!(plot.is_truncated() || plot.points == whole.points);
                    let width = plot.variables.len() * plot.numbers_per_value();
                    assert_eq!(plot.values.len(), plot.points * width, "{name} at {cut}");
                    assert_eq!(
                        plot.values,
                        whole.values[..plot.values.len()],
                        "{name} at {cut}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_cut_binary_file_keeps_its_whole_points_and_says_it_was_cut() {
        let whole = only(TRAN_BINARY);
        let row = whole.variables.len() * DOUBLE_BYTES;
        let plot = only(&TRAN_BINARY[..TRAN_BINARY.len() - row * 3 - 5]);
        assert!(plot.is_truncated());
        assert_eq!(plot.points, whole.points - 4);
        assert_eq!(plot.declared_points, whole.points);
    }

    #[test]
    fn a_cut_text_file_drops_a_half_written_number() {
        let text = String::from_utf8_lossy(OP_TEXT).into_owned();
        let cut = text.rfind("-1.0000").unwrap() + 4;
        let plot = only(&OP_TEXT[..cut]);
        assert!(plot.is_truncated());
        assert_eq!(plot.points, 0);
    }

    #[test]
    fn a_header_with_no_data_is_refused_as_cut_short() {
        let text = String::from_utf8_lossy(OP_TEXT).into_owned();
        let cut = text.find("Variables:\n").unwrap();
        assert_eq!(parse(&OP_TEXT[..cut]), Err(RawError::HeaderCutShort));
    }
    #[test]
    fn a_file_with_no_title_is_refused() {
        let text = String::from_utf8_lossy(OP_TEXT).replace("Title: konnect simulation\n", "");
        assert_eq!(parse(text.as_bytes()), Err(RawError::NotARawFile));
    }

    #[test]
    fn data_that_is_not_a_raw_file_is_refused() {
        assert_eq!(parse(b""), Err(RawError::NotARawFile));
        assert_eq!(parse(b"\n\n"), Err(RawError::NotARawFile));
        assert_eq!(parse(b"garbage\n"), Err(RawError::NotARawFile));
        assert_eq!(parse(&[0xff; 64]), Err(RawError::NotARawFile));
    }

    fn with(from: &str, to: &str) -> Vec<u8> {
        String::from_utf8_lossy(OP_TEXT)
            .replace(from, to)
            .into_bytes()
    }

    #[test]
    fn a_header_that_cannot_be_right_is_refused() {
        for (from, to) in [
            ("No. Variables: 3", "No. Variables: 0"),
            ("No. Variables: 3", "No. Variables: x"),
            ("No. Points: 1", "No. Points: -1"),
            ("No. Points: 1", "No. Points: 99999999999999999999999"),
            ("Flags: real", "Flags: strange"),
            ("Flags: real\n", ""),
            ("No. Points: 1\n", ""),
            ("\t1\tv(/out)\tvoltage", "\t5\tv(/out)\tvoltage"),
            ("\t1\tv(/out)\tvoltage", "\t1\tv(/out)"),
            (
                "Variables:\n\t0\tv(/in)\tvoltage\n\t1\tv(/out)\tvoltage\n\t2\ti(v1)\tcurrent\n",
                "",
            ),
            ("Dimensions", "Dimensions"),
        ] {
            if from == to {
                continue;
            }
            assert!(
                matches!(parse(&with(from, to)), Err(RawError::BadHeader { .. })),
                "{from:?} to {to:?}: {:?}",
                parse(&with(from, to))
            );
        }
    }

    #[test]
    fn an_unpadded_file_is_refused_by_name() {
        assert_eq!(
            parse(&with("Flags: real", "Flags: real unpadded")),
            Err(RawError::Unpadded)
        );
    }

    #[test]
    fn a_bad_number_before_the_end_is_an_error_and_not_a_cut() {
        for (from, to) in [
            ("2.000000000000000e+00", "banana"),
            (" 0\t3.0", " 7\t3.0"),
            ("\t2.000", "x\t2.000"),
            ("\t2.000", "2.000"),
        ] {
            assert!(
                matches!(parse(&with(from, to)), Err(RawError::BadValue { .. })),
                "{from:?}"
            );
        }
    }

    #[test]
    fn a_real_number_in_a_complex_plot_is_a_bad_value() {
        let text = String::from_utf8_lossy(AC_TEXT).replacen(",0.000000000000000e+00", "", 1);
        assert!(matches!(
            parse(text.as_bytes()),
            Err(RawError::BadValue { .. })
        ));
    }

    #[test]
    fn a_declared_size_larger_than_the_file_cannot_allocate_it() {
        let huge = usize::MAX.to_string();
        for bytes in [OP_TEXT, OP_BINARY] {
            let text = String::from_utf8_lossy(bytes)
                .replace("No. Points: 1", &format!("No. Points: {huge}"));
            let lossy = text.as_bytes();
            if let Ok(plots) = parse(lossy) {
                assert!(plots[0].is_truncated());
            }
        }
        let mut wide = b"Title: t\nFlags: real\nNo. Variables: 2305843009213693952\nNo. Points: 1\nVariables:\n".to_vec();
        wide.extend_from_slice(b"Binary:\n");
        assert!(parse(&wide).is_err());
    }

    #[test]
    fn windows_line_endings_read_the_same() {
        let crlf = String::from_utf8_lossy(OP_TEXT).replace('\n', "\r\n");
        assert_eq!(only(crlf.as_bytes()), only(OP_TEXT));
    }

    #[test]
    fn not_a_number_and_infinity_survive() {
        let text = with("2.000000000000000e+00", "nan");
        let plot = only(&text);
        assert!(plot.real(1).next().unwrap().is_nan());
        let text = with("2.000000000000000e+00", "-inf");
        assert_eq!(only(&text).real(1).next(), Some(f64::NEG_INFINITY));
    }

    #[test]
    fn two_plots_in_one_file_are_both_read() {
        for first in [OP_TEXT, OP_BINARY] {
            let mut joined = first.to_vec();
            joined.extend_from_slice(AC_BINARY);
            let plots = parse(&joined).unwrap();
            assert_eq!(plots.len(), 2);
            assert_eq!(plots[1].name, "AC Analysis");
            assert_eq!(plots[1].points, only(AC_BINARY).points);
        }
    }

    #[test]
    fn junk_after_a_plot_is_refused() {
        let mut joined = OP_BINARY.to_vec();
        joined.extend_from_slice(b"junk\n");
        assert_eq!(parse(&joined), Err(RawError::NotARawFile));
    }

    #[test]
    fn nested_sweep_dimensions_are_kept() {
        let text = with("No. Points: 1\n", "No. Points: 1\nDimensions: 1,1\n");
        assert_eq!(only(&text).dimensions, [1, 1]);
        assert!(only(OP_TEXT).dimensions.is_empty());
    }

    #[test]
    fn extra_header_lines_and_variable_attributes_are_ignored() {
        let text = with(
            "\nVariables:",
            "\nOption: reltol = 1e-3\nCommand: version 44\nVariables:",
        );
        assert_eq!(only(&text).variables.len(), 3);
        let text = with(
            "\tvoltage\n\t1",
            "\tvoltage min=0 max=5 color=red grid=3\n\t1",
        );
        assert_eq!(only(&text).variables[0].kind, "voltage");
    }

    #[test]
    fn a_header_in_another_case_still_reads() {
        let text = String::from_utf8_lossy(OP_TEXT)
            .replace("Title:", "TITLE:")
            .replace("Values:", "VALUES:");
        assert_eq!(only(text.as_bytes()).points, 1);
    }

    #[test]
    fn reading_a_file_checks_its_size_first() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("run.raw");
        std::fs::write(&path, OP_BINARY).unwrap();
        assert_eq!(read(&path, OP_BINARY.len() as u64).unwrap().len(), 1);
        assert_eq!(
            read(&path, OP_BINARY.len() as u64 - 1),
            Err(RawError::TooLarge(OP_BINARY.len() as u64))
        );
        assert!(matches!(
            read(&folder.path().join("absent.raw"), 1),
            Err(RawError::Unreadable(_))
        ));
    }
}
