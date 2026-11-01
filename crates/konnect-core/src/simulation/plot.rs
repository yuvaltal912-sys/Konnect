//! A run's signals as an SVG file.
//!
//! No plotting crate: the figure is a few rectangles, text and polylines. Text is
//! live `<text>`, which `konnect_render::svg_to_png` refuses [plan E41], so this
//! file is for a browser or an editor and not for rasterising here.
//!
//! Three things bound the output. A trace is cut to an envelope of at most two
//! points per pixel column, so the file does not grow with the run: the 281,020
//! point transient of the soft-start board draws as the same few thousand points
//! as a hundred-point one. Every string that did not come from this file is
//! escaped, cut to a length and stripped of control and bidirectional characters
//! before it reaches the document, because net names and file names are user
//! text. And every number comes out of `{:.1}` or a fixed table, so nothing else
//! can reach an attribute.

pub(crate) const MIN_WIDTH: u32 = 200;
pub(crate) const MAX_WIDTH: u32 = 1600;
pub(crate) const DEFAULT_WIDTH: u32 = 800;

const MARGIN_LEFT: f64 = 76.0;
const MARGIN_RIGHT: f64 = 18.0;
const SINGLE_PANEL_HEIGHT: f64 = 300.0;
const PANEL_HEIGHT: f64 = 180.0;
const PANEL_TITLE: f64 = 20.0;
const TICK_LABELS: f64 = 18.0;
const PANEL_GAP: f64 = 12.0;
const LINE_HEIGHT: f64 = 16.0;
const FONT: &str = "Helvetica, Arial, sans-serif";
/// Most characters of any caller-derived text drawn in one piece.
const MAX_TEXT_CHARS: usize = 64;
/// Most characters of a title, a banner line or a note, which wrap over lines.
const MAX_LONG_TEXT: usize = 400;
/// A trace of this many points or fewer per pixel column is drawn whole.
const POINTS_PER_COLUMN: usize = 2;
/// A trace this short also gets a mark at each sample, so a single point shows.
const MARKED_UP_TO: usize = 40;
const TRACE_WIDTH: f64 = 1.5;
/// How wide a character is, in em, for fitting text without a font to measure with.
/// Capitals and bold run wider than the 0.55 of lower case, so this is generous.
const CHAR_EM: f64 = 0.65;
/// A span narrower than this share of the largest value is widened to it, so that
/// noise on a large offset reads as the flat line it is, and its ticks stay apart.
const MIN_RELATIVE_SPAN: f64 = 1e-6;
/// Most characters of a tick label before it goes scientific: a sign, three digits,
/// the point, and the seven places that the narrowest span a plot allows needs.
const MAX_TICK_CHARS: usize = 12;

/// Colour-blind safe, after Okabe and Ito, with the pale yellow swapped for a
/// purple that holds up on white.
pub(crate) const CATEGORICAL: [&str; 8] = [
    "#0072B2", "#D55E00", "#009E73", "#CC79A7", "#B8860B", "#56B4E9", "#6A3D9A", "#555555",
];
/// Dark to light, for the steps of an outer sweep.
pub(crate) const RAMP: [&str; 8] = [
    "#440154", "#46327E", "#365C8D", "#277F8E", "#1FA187", "#4AC16D", "#8FD744", "#BBC400",
];

#[derive(Debug, Clone)]
pub(crate) struct Series {
    /// An index into the chart's palette.
    pub color: usize,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct Panel {
    /// Drawn over the panel. Empty for none.
    pub title: String,
    pub y_label: String,
    pub y_unit: String,
    /// Whether the unit takes an SI prefix (`mV`), which `dB` and `°` do not.
    pub y_si: bool,
    pub series: Vec<Series>,
}

#[derive(Debug, Clone)]
pub(crate) struct Axis {
    pub label: String,
    pub unit: String,
    pub log: bool,
    pub si: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Chart {
    pub title: String,
    pub banner: Vec<String>,
    pub x: Axis,
    pub panels: Vec<Panel>,
    pub palette: &'static [&'static str],
    pub legend: Vec<(usize, String)>,
    pub notes: Vec<String>,
    pub width: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct Rendered {
    pub svg: String,
    pub width: u32,
    pub height: u32,
    /// Points written into polylines, after the cut to envelopes.
    pub points_drawn: usize,
    /// Finite points in the series given.
    pub points_in: usize,
}

/// Text that is safe in an XML text node, cut to `max` characters.
pub(crate) fn escape(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (index, c) in text.chars().enumerate() {
        if index == max {
            out.push('…');
            break;
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if unsafe_char(c) => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Controls, which XML 1.0 forbids or a viewer may act on; line and paragraph
/// separators; the characters that reorder text, which could make a net name
/// read as another; and the Unicode non-characters.
fn unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FFFE}' | '\u{FFFF}')
        || ('\u{202A}'..='\u{202E}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)
        || ('\u{200E}'..='\u{200F}').contains(&c)
        || c == '\u{061C}'
        || ('\u{FDD0}'..='\u{FDEF}').contains(&c)
}

/// `text` cut to `max` characters, so nothing that came from the design can make a
/// line, or the file, as long as it likes.
fn clip(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().nth(max).is_some() {
        out.push('…');
    }
    out
}

fn text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * CHAR_EM
}

/// Break `text` at spaces so no line is much wider than `width`. A word longer
/// than a line is cut.
fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    let per_line = ((width / (size * CHAR_EM)).floor() as usize).max(8);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut length = 0usize;
    for word in text.split_whitespace() {
        let characters: Vec<char> = word.chars().collect();
        // A word longer than a line is cut into lines of its own.
        let mut rest = characters.as_slice();
        while rest.len() > per_line {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                length = 0;
            }
            lines.push(rest[..per_line].iter().collect());
            rest = &rest[per_line..];
        }
        if length > 0 && length + 1 + rest.len() > per_line {
            lines.push(std::mem::take(&mut line));
            length = 0;
        }
        if length > 0 {
            line.push(' ');
            length += 1;
        }
        line.extend(rest);
        length += rest.len();
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A number with one decimal, never `-0.0`.
fn px(value: f64) -> String {
    let text = format!("{value:.1}");
    match text == "-0.0" {
        true => "0.0".to_string(),
        false => text,
    }
}

const PREFIXES: [(f64, &str); 8] = [
    (1e9, "G"),
    (1e6, "M"),
    (1e3, "k"),
    (1.0, ""),
    (1e-3, "m"),
    (1e-6, "µ"),
    (1e-9, "n"),
    (1e-12, "p"),
];

/// The SI prefix that makes `largest` read as a number from 1 up to 1000.
fn prefix_for(largest: f64) -> (f64, &'static str) {
    if !largest.is_finite() || largest == 0.0 {
        return (1.0, "");
    }
    PREFIXES
        .iter()
        .find(|(factor, _)| largest.abs() >= *factor * 0.999_999)
        .copied()
        .unwrap_or((1e-12, "p"))
}

/// A number with a prefix of its own, such as `4.7k` or `10µ`.
pub(crate) fn si(value: f64) -> String {
    let (factor, prefix) = prefix_for(value);
    let scaled = format!("{:.3}", value / factor);
    let trimmed = scaled.trim_end_matches('0').trim_end_matches('.');
    let trimmed = if trimmed == "-0" || trimmed.is_empty() {
        "0"
    } else {
        trimmed
    };
    format!("{trimmed}{prefix}")
}

/// A number as plain decimal text, such as `0.5`, for a unit that takes no prefix.
pub(crate) fn plain(value: f64) -> String {
    let text = format!("{value:.4}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    match trimmed {
        "" | "-0" => "0".to_string(),
        other => other.to_string(),
    }
}

fn nice_step(raw: f64) -> f64 {
    let magnitude = 10f64.powf(raw.log10().floor());
    let norm = raw / magnitude;
    let nice = match norm {
        n if n <= 1.5 => 1.0,
        n if n <= 3.0 => 2.0,
        n if n <= 7.0 => 5.0,
        _ => 10.0,
    };
    nice * magnitude
}

fn linear_ticks(lo: f64, hi: f64, target: usize) -> (Vec<f64>, f64) {
    let step = nice_step((hi - lo) / target.max(1) as f64);
    if !step.is_finite() || step <= 0.0 {
        return (Vec::new(), 1.0);
    }
    let mut ticks = Vec::new();
    let mut k = (lo / step - 1e-9).ceil();
    while ticks.len() < 40 {
        let value = k * step;
        if value > hi + step * 1e-9 {
            break;
        }
        ticks.push(value);
        k += 1.0;
    }
    (ticks, step)
}

fn log_ticks(lo: f64, hi: f64, room: usize) -> Vec<f64> {
    if !(lo > 0.0 && hi >= lo && hi.is_finite()) {
        return Vec::new();
    }
    let first = lo.log10().floor() as i64;
    let last = hi.log10().ceil() as i64;
    let decades = (last - first).max(1) as usize;
    let mut ticks = Vec::new();
    // Over a decade or two the 1, 2 and 5 of each decade read better than the
    // power alone, which would leave one or two labels.
    let stride = decades.div_ceil(room.max(2) - 1).max(1) as i64;
    let mantissas: &[f64] = if decades <= 2 {
        &[1.0, 2.0, 5.0]
    } else {
        &[1.0]
    };
    // From the first power inside the range, so that a stride never steps over it.
    let mut power = lo.log10().ceil() as i64;
    while power <= last && ticks.len() < 40 {
        for mantissa in mantissas {
            let value = mantissa * 10f64.powi(power as i32);
            if value >= lo * (1.0 - 1e-9) && value <= hi * (1.0 + 1e-9) {
                ticks.push(value);
            }
        }
        power += stride;
    }
    // A range between two powers of ten, such as 3 Hz to 300 Hz at a narrow
    // width, still gets its ends.
    if ticks.is_empty() {
        ticks = if lo == hi { vec![lo] } else { vec![lo, hi] };
    }
    ticks
}

/// A value to pixel mapping along one axis, linear or by logarithm.
struct Map {
    lo: f64,
    hi: f64,
    log: bool,
    from: f64,
    to: f64,
}

impl Map {
    fn at(&self, value: f64) -> f64 {
        let (v, lo, hi) = match self.log {
            true => (value.ln(), self.lo.ln(), self.hi.ln()),
            false => (value, self.lo, self.hi),
        };
        self.from + (v - lo) / (hi - lo) * (self.to - self.from)
    }
}

/// A value that can be placed. Past this, the arithmetic of a span or a scale
/// overflows to infinity and a coordinate comes out as `NaN`, and no circuit
/// quantity is this large: a diverged run is.
const MAX_MAGNITUDE: f64 = 1e150;

fn plottable(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_MAGNITUDE
}

/// The span of the plottable values `values` gives.
fn extent(values: impl Iterator<Item = f64>, log: bool) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for value in values.filter(|v| plottable(*v) && (!log || *v > 0.0)) {
        lo = lo.min(value);
        hi = hi.max(value);
    }
    if lo > hi {
        return None;
    }
    Some((lo, hi))
}

fn widen(lo: f64, hi: f64, log: bool, share: f64) -> (f64, f64) {
    if lo == hi {
        return match log {
            true => (lo / 2.0, lo * 2.0),
            false => {
                let pad = if lo == 0.0 { 1.0 } else { lo.abs() * 0.05 };
                (lo - pad, hi + pad)
            }
        };
    }
    match log {
        true => (lo, hi),
        false => {
            let floor = lo.abs().max(hi.abs()) * MIN_RELATIVE_SPAN;
            let pad = match hi - lo < floor {
                true => floor / 2.0,
                false => (hi - lo) * share,
            };
            (lo - pad, hi + pad)
        }
    }
}

/// Cut a trace, already in pixels, to at most two points per bucket `width` pixels
/// wide, a pixel column for a width of 1: the lowest and the highest of each, in the
/// order they occur. A trace that is short enough is kept whole.
fn envelope(points: &[(f64, f64)], columns: usize, width: f64) -> Vec<(f64, f64)> {
    // A stretch is kept whole only if it is no denser than two points a column of
    // the width it spans, so that many short stretches cannot each pass as small.
    let (first, last) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.0), hi.max(p.0))
        });
    let spanned = (((last - first).max(0.0) / width).floor() as usize + 1)
        .min((columns.max(1) as f64 / width) as usize + 1);
    if points.len() <= spanned * POINTS_PER_COLUMN {
        return points.to_vec();
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    let keep = |low: usize, high: usize, out: &mut Vec<(f64, f64)>| {
        out.push(points[low.min(high)]);
        if low != high {
            out.push(points[low.max(high)]);
        }
    };
    let column_of = |x: f64| (x / width).floor() as i64;
    let mut column = column_of(points[0].0);
    let (mut low, mut high) = (0usize, 0usize);
    for (index, point) in points.iter().enumerate().skip(1) {
        if column_of(point.0) != column {
            keep(low, high, &mut out);
            column = column_of(point.0);
            (low, high) = (index, index);
            continue;
        }
        if point.1 < points[low].1 {
            low = index;
        }
        if point.1 > points[high].1 {
            high = index;
        }
    }
    keep(low, high, &mut out);
    // Real data has x moving one way, so each column is visited once. An x that
    // goes back and forth would visit columns again and again, so the bound is
    // held by thinning evenly.
    let cap = columns.max(1) * POINTS_PER_COLUMN * 2;
    if out.len() > cap {
        let step = out.len().div_ceil(cap);
        out = out.into_iter().step_by(step).collect();
    }
    out
}

struct Plotted {
    drawn: usize,
    given: usize,
}

fn trace(
    svg: &mut String,
    series: &Series,
    xs: &Map,
    ys: &Map,
    colour: &str,
    columns: usize,
) -> Plotted {
    let mut plotted = Plotted { drawn: 0, given: 0 };
    let mut run: Vec<(f64, f64)> = Vec::new();
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    for (&x, &y) in series.x.iter().zip(&series.y) {
        let usable = plottable(x) && plottable(y) && (!xs.log || x > 0.0);
        if usable {
            run.push((xs.at(x), ys.at(y)));
        } else if !run.is_empty() {
            runs.push(std::mem::take(&mut run));
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    // Data riddled with gaps would give a line for each stretch between them, and the
    // file would grow with the gaps. Past a line for each column, the gaps are bridged.
    if runs.len() > columns.max(1) / 2 {
        runs = vec![runs.into_iter().flatten().collect()];
    }
    plotted.given = runs.iter().map(Vec::len).sum();
    let marked = plotted.given <= MARKED_UP_TO;
    let cut = |width: f64| -> Vec<Vec<(f64, f64)>> {
        runs.iter()
            .map(|run| envelope(run, columns, width))
            .collect()
    };
    let mut stretches = cut(1.0);
    // However the stretches fall, a trace holds at most four points a column. If they
    // are too many together, every stretch is cut again to wider buckets, which keeps
    // the lowest and highest of each and so keeps a spike.
    let cap = columns.max(1) * POINTS_PER_COLUMN * 2;
    let mut width = 1.0;
    while stretches.iter().map(Vec::len).sum::<usize>() > cap && width <= columns as f64 {
        let total = stretches.iter().map(Vec::len).sum::<usize>();
        width *= (total as f64 / cap as f64).ceil().max(2.0);
        stretches = cut(width);
    }
    for kept in stretches {
        plotted.drawn += kept.len();
        if kept.len() > 1 {
            let list: Vec<String> = kept
                .iter()
                .map(|(x, y)| format!("{},{}", px(*x), px(*y)))
                .collect();
            svg.push_str(&format!(
                "<polyline fill=\"none\" stroke=\"{colour}\" stroke-width=\"{TRACE_WIDTH}\" \
                 stroke-linejoin=\"round\" points=\"{}\"/>\n",
                list.join(" ")
            ));
        }
        // A stretch of one point has no line, and is still data.
        if marked || kept.len() == 1 {
            for (x, y) in &kept {
                svg.push_str(&format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"2.5\" fill=\"{colour}\"/>\n",
                    px(*x),
                    px(*y)
                ));
            }
        }
    }
    plotted
}

/// What a panel's y axis shows: its span, its prefix and its ticks.
struct YAxis {
    lo: f64,
    hi: f64,
    factor: f64,
    prefix: &'static str,
    ticks: Vec<f64>,
    step: f64,
}

fn y_axis(panel: &Panel, panel_height: f64) -> YAxis {
    let span = extent(panel.series.iter().flat_map(|s| s.y.iter().copied()), false);
    let (lo, hi) = span.map_or((0.0, 1.0), |(lo, hi)| widen(lo, hi, false, 0.06));
    let (factor, prefix) = match panel.y_si {
        true => prefix_for(lo.abs().max(hi.abs())),
        false => (1.0, ""),
    };
    let (ticks, step) = linear_ticks(lo, hi, ((panel_height / 40.0) as usize).max(2));
    YAxis {
        lo,
        hi,
        factor,
        prefix,
        ticks,
        step,
    }
}

fn tick_text(value: f64, factor: f64, step: f64) -> String {
    let scaled = value / factor;
    let unit_step = step / factor;
    // Ticks a hair apart, or past a billion, would be a dozen digits wide.
    if scaled == 0.0 && unit_step < 1e-9 {
        return "0".to_string();
    }
    if scaled != 0.0 && (scaled.abs() >= 1e9 || unit_step < 1e-9) {
        return format!("{scaled:.2e}");
    }
    let places = (-unit_step.log10().floor()).clamp(0.0, 9.0) as usize;
    let text = format!("{scaled:.places$}");
    if text.chars().count() > MAX_TICK_CHARS {
        return format!("{scaled:.2e}");
    }
    match text
        .trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        true => text.trim_start_matches('-').to_string(),
        false => text,
    }
}

fn text(svg: &mut String, x: f64, y: f64, size: f64, anchor: &str, weight: &str, body: &str) {
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"{size}\" text-anchor=\"{anchor}\" \
         font-weight=\"{weight}\">{body}</text>\n",
        px(x),
        px(y)
    ));
}

/// Draw the chart.
pub(crate) fn render(chart: &Chart) -> Rendered {
    let width = chart.width.clamp(MIN_WIDTH, MAX_WIDTH);
    let w = f64::from(width);
    let panel_height = if chart.panels.len() == 1 {
        SINGLE_PANEL_HEIGHT
    } else {
        PANEL_HEIGHT
    };
    let y_axes: Vec<YAxis> = chart
        .panels
        .iter()
        .map(|panel| y_axis(panel, panel_height))
        .collect();
    // The left margin holds the y tick labels, however long they are, and the
    // rotated axis label beyond them.
    let widest = y_axes
        .iter()
        .flat_map(|axis| {
            let label = |tick: &f64| tick_text(*tick, axis.factor, axis.step).chars().count();
            axis.ticks.iter().map(label)
        })
        .max()
        .unwrap_or(0);
    let left = (28.0 + widest as f64 * 11.0 * CHAR_EM + 6.0).max(MARGIN_LEFT);
    let area = (w - left - MARGIN_RIGHT).max(40.0);
    let columns = area.floor() as usize;
    let mut body = String::new();
    let mut y = 0.0;

    // The banner first, so that nothing hides it.
    if !chart.banner.is_empty() {
        let lines: Vec<String> = chart
            .banner
            .iter()
            .flat_map(|line| wrap(&clip(line, MAX_LONG_TEXT), 12.0, w - 24.0))
            .collect();
        let height = 12.0 + LINE_HEIGHT * lines.len() as f64;
        body.push_str(&format!(
            "<rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{}\" fill=\"#FFF3CD\" \
             stroke=\"#B58100\" stroke-width=\"2\"/>\n",
            px(height)
        ));
        for (index, line) in lines.iter().enumerate() {
            let at = 6.0 + LINE_HEIGHT * (index as f64 + 0.8);
            let tag = format!(
                "<text x=\"12\" y=\"{}\" font-size=\"12\" font-weight=\"bold\" \
                 fill=\"#4A3000\">{}</text>\n",
                px(at),
                escape(line, usize::MAX)
            );
            body.push_str(&tag);
        }
        y += height + 6.0;
    }

    let title_lines = wrap(
        &escape_plain(&clip(&chart.title, MAX_LONG_TEXT)),
        14.0,
        w - 24.0,
    );
    for line in &title_lines {
        y += 18.0;
        text(
            &mut body,
            12.0,
            y,
            14.0,
            "start",
            "bold",
            &escape(line, usize::MAX),
        );
    }
    y += 6.0;

    // Legend, wrapped to the width, each entry short enough to fit one line.
    if !chart.legend.is_empty() {
        let legend_chars = (((w - 50.0) / (12.0 * CHAR_EM)) as usize).clamp(8, MAX_TEXT_CHARS);
        let (mut x, mut row) = (12.0, y);
        for (colour, label) in &chart.legend {
            let label = escape(&escape_plain(label), legend_chars);
            let entry = 24.0 + text_width(&label, 12.0) + 14.0;
            if x + entry > w && x > 12.0 {
                x = 12.0;
                row += LINE_HEIGHT;
            }
            let colour = chart.palette[colour % chart.palette.len()];
            body.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{colour}\" \
                 stroke-width=\"3\"/>\n",
                px(x),
                px(row + 9.0),
                px(x + 16.0),
                px(row + 9.0)
            ));
            text(
                &mut body,
                x + 22.0,
                row + 13.0,
                12.0,
                "start",
                "normal",
                &label,
            );
            x += entry;
        }
        y = row + LINE_HEIGHT + 8.0;
    }

    // The x range is shared by every panel.
    let log = chart.x.log;
    let data_x = chart
        .panels
        .iter()
        .flat_map(|panel| &panel.series)
        .flat_map(|series| series.x.iter().copied());
    let x_span = extent(data_x, log);
    let empty_span = if log { (1.0, 10.0) } else { (0.0, 1.0) };
    let (x_lo, x_hi) = x_span.map_or(empty_span, |(lo, hi)| widen(lo, hi, log, 0.0));
    // Linear data fills the plot edge to edge, so a line meets its frame.
    let xs = Map {
        lo: x_lo,
        hi: x_hi,
        log,
        from: left,
        to: left + area,
    };
    let (x_factor, x_prefix) = match (chart.x.si, log) {
        (true, false) => prefix_for(x_lo.abs().max(x_hi.abs())),
        _ => (1.0, ""),
    };
    let x_label_of = |tick: f64, step: f64| match log {
        true => si(tick),
        false => tick_text(tick, x_factor, step),
    };
    let (x_ticks, x_step) = match log {
        true => (log_ticks(x_lo, x_hi, (area / 70.0) as usize), 1.0),
        false => {
            let mut wanted = ((area / 90.0) as usize).max(2);
            let mut ticks = linear_ticks(x_lo, x_hi, wanted);
            // Wide labels, such as nine digits on a hair of a span, need more room.
            let widest = |(ticks, step): &(Vec<f64>, f64)| {
                let width = |tick: &f64| x_label_of(*tick, *step).chars().count();
                ticks.iter().map(width).max().unwrap_or(0) as f64 * 11.0 * CHAR_EM
            };
            let need = widest(&ticks) * 1.3;
            if need > area / ticks.0.len().max(1) as f64 {
                wanted = ((area / need) as usize).max(1);
                ticks = linear_ticks(x_lo, x_hi, wanted);
            }
            ticks
        }
    };

    let (mut points_drawn, mut points_in) = (0usize, 0usize);
    for (panel, axis) in chart.panels.iter().zip(&y_axes) {
        if !panel.title.is_empty() {
            // One character less, for the ellipsis a cut adds.
            let fits = (((w - left) / (12.0 * CHAR_EM)) as usize)
                .saturating_sub(1)
                .clamp(8, MAX_TEXT_CHARS);
            let title = escape(&panel.title, fits);
            text(&mut body, left, y + 14.0, 12.0, "start", "bold", &title);
        }
        let top = y + PANEL_TITLE;
        let bottom = top + panel_height;
        let (y_lo, y_hi) = (axis.lo, axis.hi);
        let ys = Map {
            lo: y_lo,
            hi: y_hi,
            log: false,
            from: bottom,
            to: top,
        };
        let (y_factor, y_prefix, y_step) = (axis.factor, axis.prefix, axis.step);
        let y_ticks = &axis.ticks;

        body.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#FFFFFF\" \
             stroke=\"#8A8A8A\"/>\n",
            px(left),
            px(top),
            px(area),
            px(panel_height)
        ));
        for tick in y_ticks {
            let at = ys.at(*tick);
            body.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#E2E2E2\"/>\n",
                px(left),
                px(at),
                px(left + area),
                px(at)
            ));
            text(
                &mut body,
                left - 5.0,
                at + 4.0,
                11.0,
                "end",
                "normal",
                &tick_text(*tick, y_factor, y_step),
            );
        }
        for tick in &x_ticks {
            let at = xs.at(*tick);
            body.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#E2E2E2\"/>\n",
                px(at),
                px(top),
                px(at),
                px(bottom)
            ));
            let label = x_label_of(*tick, x_step);
            // A label that would run off the right edge ends at it instead.
            let half = label.chars().count() as f64 * 11.0 * CHAR_EM / 2.0;
            let (anchor, place) = match at + half > w - 2.0 {
                true => ("end", w - 2.0),
                false => ("middle", at),
            };
            text(
                &mut body,
                place,
                bottom + 14.0,
                11.0,
                anchor,
                "normal",
                &label,
            );
        }
        if panel
            .series
            .iter()
            .all(|s| s.y.iter().all(|v| !v.is_finite()))
        {
            text(
                &mut body,
                left + area / 2.0,
                top + panel_height / 2.0,
                12.0,
                "middle",
                "normal",
                "no data",
            );
        }
        for series in &panel.series {
            let colour = chart.palette[series.color % chart.palette.len()];
            let plotted = trace(&mut body, series, &xs, &ys, colour, columns);
            points_drawn += plotted.drawn;
            points_in += plotted.given;
        }
        let unit = format!("{y_prefix}{}", panel.y_unit);
        // The label runs up the panel, so the panel's height is what it must fit.
        let unit = escape(&unit, 16);
        let room = (panel_height / (12.0 * CHAR_EM)) as usize;
        let name = room.saturating_sub(unit.chars().count() + 4).clamp(4, 40);
        let label = format!("{} ({unit})", escape(&panel.y_label, name));
        let (cx, cy) = (14.0, top + panel_height / 2.0);
        body.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"12\" text-anchor=\"middle\" \
             transform=\"rotate(-90 {} {})\">{label}</text>\n",
            px(cx),
            px(cy),
            px(cx),
            px(cy)
        ));
        y = bottom + TICK_LABELS + PANEL_GAP;
    }

    let x_unit = format!("{x_prefix}{}", chart.x.unit);
    // Under the plot the label must fit the picture.
    // It is centred on the plot, so it has as much room as the nearer edge allows.
    let centre = left + area / 2.0;
    let half = centre.min(w - centre);
    let across = (((2.0 * half - 8.0) / (12.0 * CHAR_EM)) as usize).clamp(8, MAX_TEXT_CHARS + 16);
    let x_label = match x_unit.is_empty() {
        true => escape(&chart.x.label, across.min(MAX_TEXT_CHARS)),
        false => {
            let unit = escape(&x_unit, 16);
            let name = across.saturating_sub(unit.chars().count() + 4).clamp(4, 40);
            format!("{} ({unit})", escape(&chart.x.label, name))
        }
    };
    y += 4.0;
    text(
        &mut body,
        left + area / 2.0,
        y + 8.0,
        12.0,
        "middle",
        "normal",
        &x_label,
    );
    y += 22.0;
    for note in &chart.notes {
        for line in wrap(&escape_plain(&clip(note, MAX_LONG_TEXT)), 11.0, w - 24.0) {
            text(
                &mut body,
                12.0,
                y + 11.0,
                11.0,
                "start",
                "normal",
                &escape(&line, usize::MAX),
            );
            y += 14.0;
        }
    }
    y += 8.0;
    let height = y.ceil() as u32;

    let label = escape(&chart.title, 120);
    let mut svg = String::with_capacity(body.len() + 400);
    svg.push_str(&format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\" font-family=\"{FONT}\" fill=\"#222222\" \
         role=\"img\" aria-label=\"{label}\">\n\
         <title>{label}</title>\n\
         <rect width=\"{width}\" height=\"{height}\" fill=\"#FFFFFF\"/>\n"
    ));
    svg.push_str(&body);
    svg.push_str("</svg>\n");
    Rendered {
        svg,
        width,
        height,
        points_drawn,
        points_in,
    }
}

/// Text meant to be escaped once, later: drops what `escape` would turn to a
/// space so that `wrap` measures the same text it will draw.
fn escape_plain(text: &str) -> String {
    text.chars()
        .map(|c| if unsafe_char(c) { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use usvg::roxmltree::Document;

    fn line(color: usize, x: Vec<f64>, y: Vec<f64>) -> Series {
        Series { color, x, y }
    }

    fn panel(series: Vec<Series>) -> Panel {
        Panel {
            title: String::new(),
            y_label: "Voltage".into(),
            y_unit: "V".into(),
            y_si: true,
            series,
        }
    }

    fn chart(panels: Vec<Panel>) -> Chart {
        Chart {
            title: "Transient".into(),
            banner: Vec::new(),
            x: Axis {
                label: "Time".into(),
                unit: "s".into(),
                log: false,
                si: true,
            },
            panels,
            palette: &CATEGORICAL,
            legend: vec![(0, "v(out)".into())],
            notes: Vec::new(),
            width: 800,
        }
    }

    /// A sine of `points` samples over one second.
    fn sine(points: usize) -> Series {
        let x: Vec<f64> = (0..points).map(|i| i as f64 / points as f64).collect();
        let y = x
            .iter()
            .map(|t| (t * 2.0 * std::f64::consts::PI * 50.0).sin())
            .collect();
        line(0, x, y)
    }

    /// Every text node in the document, so a test sees what a viewer would draw.
    fn texts(svg: &str) -> Vec<String> {
        let document = Document::parse(svg).expect("the plot is well-formed XML");
        document
            .descendants()
            .filter(|node| node.has_tag_name("text"))
            .map(|node| node.text().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn text_is_escaped_cut_and_stripped_of_controls_and_reordering_marks() {
        assert_eq!(escape("a<b>&\"'", 64), "a&lt;b&gt;&amp;&quot;&#39;");
        assert_eq!(escape("a\u{0}b\u{7}c\nd\te", 64), "a b c d e");
        // Reordering marks could make one net name read as another.
        assert_eq!(escape("v\u{202E}ba\u{2066}r", 64), "v ba r");
        assert_eq!(escape("a\u{2028}b\u{FFFF}c\u{FDD0}d", 64), "a b c d");
        assert_eq!(escape("a\u{200E}b\u{200F}c", 64), "a b c");
        assert_eq!(escape("abcdef", 3), "abc…");
        assert_eq!(escape("abc", 3), "abc");
        // Cut by characters, never in the middle of one.
        assert_eq!(escape("ééé", 2), "éé…");
    }

    #[test]
    fn hostile_names_reach_every_text_place_as_text_and_the_file_still_parses() {
        let hostile = "</text><script>alert(1)</script><g onload=\"x()\"> & \u{0}\u{1b}[31m";
        let mut chart = chart(vec![Panel {
            title: hostile.into(),
            y_label: hostile.into(),
            y_unit: hostile.into(),
            y_si: false,
            series: vec![sine(100)],
        }]);
        chart.title = hostile.into();
        chart.banner = vec![hostile.into()];
        chart.x.label = hostile.into();
        chart.x.unit = hostile.into();
        chart.legend = vec![(0, hostile.into())];
        chart.notes = vec![hostile.into()];
        let svg = render(&chart).svg;
        let document = Document::parse(&svg).expect("well-formed");
        // No element or attribute came from the text.
        for node in document.descendants().filter(|n| n.is_element()) {
            let name = node.tag_name().name();
            assert!(
                ["svg", "title", "rect", "text", "line", "polyline", "circle"].contains(&name),
                "unexpected element {name}"
            );
            for attribute in node.attributes() {
                assert!(
                    !attribute.name().starts_with("on"),
                    "attribute {}",
                    attribute.name()
                );
            }
        }
        assert!(!svg.contains("<script"), "{svg}");
        assert!(!svg.contains('\u{0}') && !svg.contains('\u{1b}'));
        // The text shows up as text, so a reader sees what was written.
        let drawn = texts(&svg).join("\n");
        assert!(
            drawn.contains("</text><script>alert(1)</script>"),
            "{drawn}"
        );
    }

    #[test]
    fn a_long_run_is_cut_to_a_bounded_envelope_and_the_file_does_not_grow_with_it() {
        let small = render(&chart(vec![panel(vec![sine(20_000)])]));
        let big = render(&chart(vec![panel(vec![sine(281_020)])]));
        assert_eq!(big.points_in, 281_020);
        // Two points a pixel column at most: 706 columns at 800 px, and the last edge.
        assert!(big.points_drawn <= 2 * 707, "{}", big.points_drawn);
        assert!(big.svg.len() < 40_000, "{} bytes", big.svg.len());
        // The same bound whether the run had 20 thousand points or 281 thousand.
        let (a, b) = (small.svg.len() as f64, big.svg.len() as f64);
        assert!((a - b).abs() / a < 0.25, "{a} against {b}");
        // Eight signals at the widest stay under half a megabyte.
        let series: Vec<Series> = (0..8)
            .map(|i| Series {
                color: i,
                ..sine(281_020)
            })
            .collect();
        let mut wide = chart(vec![panel(series)]);
        wide.width = MAX_WIDTH;
        let widest = render(&wide);
        assert!(widest.svg.len() < 500_000, "{} bytes", widest.svg.len());
    }

    #[test]
    fn a_narrow_spike_on_a_flat_trace_survives_the_cut() {
        let n = 281_020;
        let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mut y = vec![0.0; n];
        y[123_457] = 10.0;
        y[200_001] = -5.0;
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        // The panel spans -5.75 to 10.75 or so; a spike that was dropped would
        // leave a flat line, whose polyline holds one y value.
        let document = Document::parse(&drawn.svg).unwrap();
        let points = document
            .descendants()
            .find(|n| n.has_tag_name("polyline"))
            .and_then(|n| n.attribute("points"))
            .unwrap();
        let ys: Vec<f64> = points
            .split(' ')
            .map(|p| p.split(',').nth(1).unwrap().parse().unwrap())
            .collect();
        let (top, bottom) = (
            ys.iter().cloned().fold(f64::INFINITY, f64::min),
            ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        );
        assert!(bottom - top > 200.0, "the spikes are gone: {top} {bottom}");
    }

    #[test]
    fn a_short_trace_is_kept_whole_with_a_mark_at_each_sample() {
        let drawn = render(&chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0, 2.0],
            vec![0.0, 1.0, 0.5],
        )])]));
        assert_eq!(drawn.points_drawn, 3);
        assert_eq!(drawn.svg.matches("<circle").count(), 3);
        // One point has no line, and is still there.
        let one = render(&chart(vec![panel(vec![line(0, vec![1.0], vec![2.0])])]));
        assert_eq!(one.svg.matches("<circle").count(), 1);
        assert!(!one.svg.contains("<polyline"));
        // A trace of a thousand points is a line alone.
        assert_eq!(
            render(&chart(vec![panel(vec![sine(1000)])]))
                .svg
                .matches("<circle")
                .count(),
            0
        );
    }

    #[test]
    fn values_that_are_not_numbers_break_the_line_and_never_reach_the_file() {
        let drawn = render(&chart(vec![panel(vec![line(
            0,
            (0..6).map(f64::from).collect(),
            vec![0.0, 1.0, f64::NAN, f64::INFINITY, 2.0, 3.0],
        )])]));
        assert!(!drawn.svg.contains("NaN") && !drawn.svg.contains("inf"));
        assert_eq!(drawn.svg.matches("<polyline").count(), 2);
        assert_eq!(drawn.points_in, 4);
        // A panel with nothing finite says so, and still parses.
        let none = render(&chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0],
            vec![f64::NAN; 2],
        )])]));
        assert!(texts(&none.svg).iter().any(|t| t == "no data"));
    }

    #[test]
    fn a_banner_is_drawn_first_with_every_line_and_not_at_all_without_lines() {
        let mut banner = chart(vec![panel(vec![sine(100)])]);
        banner.banner = vec![
            "PARTIAL RUN: stopped early".into(),
            "MODIFIED CIRCUIT".into(),
        ];
        let with = render(&banner);
        let shown = texts(&with.svg);
        assert!(shown.iter().any(|t| t.contains("PARTIAL RUN")), "{shown:?}");
        assert!(shown.iter().any(|t| t.contains("MODIFIED CIRCUIT")));
        // It comes ahead of the title and the plot, so nothing covers it.
        let at = |needle: &str| with.svg.find(needle).unwrap();
        assert!(at("#FFF3CD") < at(">Transient</text>"));
        assert!(at("PARTIAL RUN") < at("<polyline"));
        let without = render(&chart(vec![panel(vec![sine(100)])]));
        assert!(!without.svg.contains("#FFF3CD"));
        assert!(with.height > without.height);
    }

    #[test]
    fn a_long_banner_line_wraps_inside_the_width() {
        let mut narrow = chart(vec![panel(vec![sine(100)])]);
        narrow.width = MIN_WIDTH;
        narrow.banner = vec![
            "MODIFIED CIRCUIT: this run simulated a modified circuit, with parts \
                              left out and lines added, and not the schematic as drawn."
                .into(),
        ];
        let drawn = render(&narrow);
        let widest = texts(&drawn.svg)
            .iter()
            .filter(|t| t.chars().count() > 10)
            .map(|t| text_width(t, 12.0))
            .fold(0.0, f64::max);
        assert!(widest <= f64::from(MIN_WIDTH), "{widest}");
        assert!(
            texts(&drawn.svg)
                .iter()
                .filter(|t| t.contains("MODIFIED") || t.contains("schematic"))
                .count()
                >= 1
        );
    }

    #[test]
    fn ticks_are_round_numbers_and_the_prefix_follows_the_size() {
        let (ticks, step) = linear_ticks(0.0, 1.0, 5);
        assert_eq!(step, 0.2);
        assert_eq!(ticks.len(), 6);
        assert_eq!(
            linear_ticks(-3.0, 3.0, 4).0,
            [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0]
        );
        assert_eq!(tick_text(-0.0, 1.0, 0.5), "0.0");
        assert_eq!(tick_text(-0.0, 1.0, 1.0), "0");
        assert_eq!(tick_text(0.0015, 1e-3, 0.5e-3), "1.5");
        assert_eq!(prefix_for(0.014), (1e-3, "m"));
        assert_eq!(prefix_for(4700.0), (1e3, "k"));
        assert_eq!(prefix_for(0.0), (1.0, ""));
        assert_eq!(si(4700.0), "4.7k");
        assert_eq!(si(1e-5), "10µ");
        assert_eq!(si(0.0), "0");
        assert_eq!(si(100.0), "100");
        assert_eq!(log_ticks(1.0, 1e6, 8), [1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6]);
        assert_eq!(
            log_ticks(1.0, 100.0, 8),
            [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]
        );
        // A wide sweep skips decades rather than crowd the labels.
        assert!(log_ticks(1e-3, 1e9, 6).len() <= 6);
    }

    #[test]
    fn a_log_axis_drops_what_has_no_logarithm_and_labels_decades() {
        let mut bode = chart(vec![panel(vec![line(
            0,
            vec![-1.0, 0.0, 1.0, 10.0, 100.0, 1000.0],
            vec![0.0; 6],
        )])]);
        bode.x = Axis {
            label: "Frequency".into(),
            unit: "Hz".into(),
            log: true,
            si: false,
        };
        let drawn = render(&bode);
        assert_eq!(
            drawn.points_in, 4,
            "zero and negative frequency are dropped"
        );
        assert!(!drawn.svg.contains("NaN") && !drawn.svg.contains("inf"));
        // The axis starts at the first frequency that has a logarithm, at the left edge.
        let first = drawn.svg.find("<polyline").unwrap();
        assert!(
            drawn.svg[first..].contains("points=\"76.0,"),
            "{}",
            &drawn.svg[first..]
        );
        let shown = texts(&drawn.svg);
        for label in ["1", "10", "100", "1k", "Frequency (Hz)"] {
            assert!(shown.iter().any(|t| t == label), "{label} in {shown:?}");
        }
    }

    #[test]
    fn axes_name_their_unit_with_the_prefix_the_numbers_were_divided_by() {
        let x: Vec<f64> = (0..50).map(|i| f64::from(i) * 1e-4).collect();
        let y: Vec<f64> = x.iter().map(|t| 0.012 + t).collect();
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        let shown = texts(&drawn.svg);
        assert!(shown.iter().any(|t| t == "Time (ms)"), "{shown:?}");
        assert!(shown.iter().any(|t| t == "Voltage (mV)"), "{shown:?}");
    }

    #[test]
    fn the_width_is_held_to_the_range_and_the_height_follows_the_content() {
        let mut narrow = chart(vec![panel(vec![sine(100)])]);
        narrow.width = 10;
        assert_eq!(render(&narrow).width, MIN_WIDTH);
        narrow.width = 99_999;
        assert_eq!(render(&narrow).width, MAX_WIDTH);
        let one = render(&chart(vec![panel(vec![sine(100)])]));
        let three = render(&chart(vec![
            panel(vec![sine(100)]),
            panel(vec![sine(100)]),
            panel(vec![sine(100)]),
        ]));
        assert!(three.height > one.height + 300);
        let document = Document::parse(&one.svg).unwrap();
        let root = document.root_element();
        assert_eq!(root.attribute("width"), Some("800"));
        assert_eq!(
            root.attribute("height"),
            Some(one.height.to_string().as_str())
        );
    }

    #[test]
    fn a_flat_trace_and_an_empty_chart_still_draw() {
        let flat = render(&chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0],
            vec![5.0, 5.0],
        )])]));
        assert!(flat.svg.contains("<polyline"));
        let empty = render(&chart(Vec::new()));
        assert!(Document::parse(&empty.svg).is_ok());
    }

    #[test]
    fn a_word_longer_than_the_line_is_cut_and_never_run_past_the_edge() {
        let lines = wrap(
            "a abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ b",
            12.0,
            100.0,
        );
        let per_line = (100.0 / (12.0 * CHAR_EM)).floor() as usize;
        assert!(
            lines.iter().all(|l| l.chars().count() <= per_line),
            "{lines:?}"
        );
        assert!(lines.len() >= 3, "{lines:?}");
        // Spaces break lines where they can.
        let words = wrap("one two three four five six seven eight", 12.0, 100.0);
        assert!(
            words.len() >= 3 && words.iter().all(|l| l.chars().count() <= per_line),
            "{words:?}"
        );
        assert_eq!(wrap("", 12.0, 100.0), Vec::<String>::new());
    }

    #[test]
    fn coordinates_never_read_minus_zero() {
        assert_eq!(px(-0.04), "0.0");
        assert_eq!(px(-0.0), "0.0");
        assert_eq!(px(12.34), "12.3");
        assert_eq!(px(-1.26), "-1.3");
    }

    #[test]
    fn values_too_large_to_scale_are_left_out_and_never_make_a_coordinate_nan() {
        let drawn = render(&chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0, 2.0, 3.0],
            vec![0.0, 1e300, -1e308, 2.0],
        )])]));
        assert!(!drawn.svg.contains("NaN") && !drawn.svg.contains("inf"));
        assert_eq!(drawn.points_in, 2);
        // The axis is that of what remains: 0 to 2, not 1e300.
        assert!(texts(&drawn.svg).iter().any(|t| t == "Voltage (V)"));
        let wide = render(&chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0],
            vec![1e149, -1e149],
        )])]));
        assert!(!wide.svg.contains("NaN") && wide.points_in == 2);
    }

    #[test]
    fn a_log_axis_with_nothing_to_place_draws_an_empty_frame_and_never_panics() {
        for xs in [vec![], vec![0.0], vec![-3.0, -1.0], vec![f64::NAN]] {
            let ys = vec![1.0; xs.len()];
            let mut empty = chart(vec![panel(vec![line(0, xs, ys)])]);
            empty.x.log = true;
            empty.x.si = false;
            let drawn = render(&empty);
            assert!(!drawn.svg.contains("NaN") && !drawn.svg.contains("inf"));
            assert!(Document::parse(&drawn.svg).is_ok());
            // An empty frame still has an axis to read: one decade.
            let shown = texts(&drawn.svg);
            assert!(shown.iter().any(|t| t == "10"), "{shown:?}");
        }
        // One frequency is a range of one: it gets a span of its own.
        let mut one = chart(vec![panel(vec![line(0, vec![10.0, 10.0], vec![1.0, 2.0])])]);
        one.x.log = true;
        one.x.si = false;
        let drawn = render(&one);
        assert!(!drawn.svg.contains("NaN"));
        assert!(
            texts(&drawn.svg).iter().any(|t| t == "10"),
            "{:?}",
            texts(&drawn.svg)
        );
    }

    #[test]
    fn a_narrow_log_axis_between_two_powers_still_has_labels() {
        let mut narrow = chart(vec![panel(vec![line(
            0,
            vec![3.16, 316_000.0],
            vec![0.0, 1.0],
        )])]);
        narrow.width = MIN_WIDTH;
        narrow.x.log = true;
        narrow.x.si = false;
        narrow.x.label = "Frequency".into();
        narrow.x.unit = "Hz".into();
        let shown = texts(&render(&narrow).svg);
        let numbers = shown
            .iter()
            .filter(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .count();
        assert!(numbers >= 2, "{shown:?}");
        // Ticks come from the first power inside the range and keep to it.
        for tick in log_ticks(3.16, 316_000.0, 1) {
            assert!((3.16..=316_000.0).contains(&tick), "{tick}");
        }
        assert_eq!(log_ticks(3.0, 300.0, 1), [10.0]);
        assert_eq!(log_ticks(0.0, 10.0, 4), Vec::<f64>::new());
        assert_eq!(log_ticks(5.0, 5.0, 4), [5.0]);
    }

    #[test]
    fn a_long_label_keeps_its_unit_and_long_text_is_cut_to_a_length() {
        let mut long = chart(vec![Panel {
            title: String::new(),
            y_label: "L".repeat(200),
            y_unit: "V".into(),
            y_si: false,
            series: vec![sine(10)],
        }]);
        long.x.label = "T".repeat(200);
        long.title = "N".repeat(100_000);
        long.banner = vec!["B".repeat(100_000)];
        long.notes = vec!["M".repeat(100_000)];
        let drawn = render(&long);
        assert!(drawn.svg.len() < 20_000, "{} bytes", drawn.svg.len());
        let shown = texts(&drawn.svg);
        assert!(
            shown
                .iter()
                .any(|t| t.starts_with("LLL") && t.ends_with("… (V)")),
            "{shown:?}"
        );
        assert!(
            shown
                .iter()
                .any(|t| t.starts_with("TTT") && t.ends_with("… (ms)")),
            "{shown:?}"
        );
    }

    #[test]
    fn a_legend_entry_is_cut_to_fit_the_narrowest_picture() {
        let mut narrow = chart(vec![panel(vec![sine(10)])]);
        narrow.width = MIN_WIDTH;
        narrow.legend = vec![(0, "v".repeat(64))];
        let shown = texts(&render(&narrow).svg);
        let entry = shown.iter().find(|t| t.starts_with("vvv")).unwrap();
        assert!(
            24.0 + text_width(entry, 12.0) <= f64::from(MIN_WIDTH),
            "{entry}"
        );
    }

    #[test]
    fn a_trace_whose_x_goes_back_and_forth_is_still_bounded() {
        // Two columns, visited 100,000 times each.
        let x: Vec<f64> = (0..200_000)
            .map(|i| if i % 2 == 0 { 0.0 } else { 700.0 })
            .collect();
        let y: Vec<f64> = (0..200_000).map(|i| f64::from(i % 7)).collect();
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        assert!(drawn.points_drawn <= 4 * 707, "{}", drawn.points_drawn);
        assert!(drawn.svg.len() < 100_000, "{} bytes", drawn.svg.len());
    }

    #[test]
    fn wrapping_a_huge_word_takes_time_in_proportion_to_its_length() {
        let started = std::time::Instant::now();
        let lines = wrap(&"w".repeat(400_000), 12.0, 100.0);
        assert!(lines.len() > 30_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_ramp_gives_each_of_its_eight_steps_a_colour_of_its_own() {
        let distinct: std::collections::BTreeSet<&&str> = RAMP.iter().collect();
        assert_eq!(distinct.len(), 8);
        assert_eq!(plain(0.5), "0.5");
        assert_eq!(plain(-0.0), "0");
        assert_eq!(plain(100.0), "100");
        assert_eq!(plain(-2.25), "-2.25");
        assert_eq!(escape("a\u{061C}b", 64), "a b");
    }

    #[test]
    fn data_riddled_with_gaps_is_still_bounded_and_every_stretch_still_shows() {
        let n = 300_000;
        let x: Vec<f64> = (0..n).map(f64::from).collect();
        let every_third: Vec<f64> = (0..n)
            .map(|i| {
                if i % 3 == 0 {
                    f64::NAN
                } else {
                    f64::from(i % 5)
                }
            })
            .collect();
        let drawn = render(&chart(vec![panel(vec![line(0, x.clone(), every_third)])]));
        assert!(drawn.svg.len() < 200_000, "{} bytes", drawn.svg.len());
        // Every other sample gone: a stretch of one point is data, and is drawn.
        let every_second: Vec<f64> = (0..n)
            .map(|i| if i % 2 == 0 { f64::NAN } else { 1.0 })
            .collect();
        let sparse = render(&chart(vec![panel(vec![line(0, x, every_second)])]));
        assert!(sparse.svg.len() < 200_000, "{} bytes", sparse.svg.len());
        assert!(sparse.svg.contains("<polyline"));
        // A few isolated points are marks, since a line of one point is nothing.
        let few = render(&chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            vec![1.0, f64::NAN, 2.0, f64::NAN, 3.0],
        )])]));
        assert_eq!(few.svg.matches("<circle").count(), 3);
    }

    #[test]
    fn a_tick_label_is_never_wider_than_a_few_characters_whatever_the_span() {
        // A 5 V rail with a femtoamp of noise: ticks a hair apart.
        assert!(tick_text(5.000000000000001, 1.0, 1e-15).len() <= 10);
        assert_eq!(tick_text(0.0, 1.0, 1e-15), "0");
        // A value near the limit of what is plotted.
        assert!(tick_text(1e149, 1e9, 1e148).len() <= 10);
        assert_eq!(tick_text(-17.3, 1e-3, 5.0), "-17300");
        // Past nine characters a label goes scientific rather than crowd the axis.
        assert_eq!(tick_text(123_456.123, 1.0, 0.001), "123456.123");
        assert_eq!(tick_text(123_456.123_456_7, 1.0, 1e-7), "1.23e5");
        assert_eq!(tick_text(12_345.678, 1.0, 0.001), "12345.678");
        assert_eq!(tick_text(12_345.678_9, 1.0, 0.0001), "12345.6789");
        assert_eq!(tick_text(1234.567, 1.0, 0.001), "1234.567");
        // Ordinary ticks keep their places.
        assert_eq!(tick_text(0.25, 1.0, 0.05), "0.25");
        assert_eq!(tick_text(2.0, 1.0, 1.0), "2");
        let mut offset = chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0, 2.0],
            vec![5.0, 5.0 + 1e-15, 5.0],
        )])]);
        offset.width = MIN_WIDTH;
        let drawn = render(&offset);
        assert!(
            texts(&drawn.svg).iter().all(|t| t.len() < 40),
            "{:?}",
            texts(&drawn.svg)
        );
    }

    /// Where a text sits, as a viewer would put it, with the width estimated at
    /// `CHAR_EM` a character: its left, right, top and bottom.
    fn text_boxes(svg: &str) -> Vec<(String, f64, f64, f64, f64)> {
        let document = Document::parse(svg).unwrap();
        document
            .descendants()
            .filter(|node| node.has_tag_name("text"))
            .map(|node| {
                let number = |name: &str| node.attribute(name).and_then(|v| v.parse::<f64>().ok());
                let text = node.text().unwrap_or_default().to_string();
                let (x, y, size) = (
                    number("x").unwrap(),
                    number("y").unwrap(),
                    number("font-size").unwrap(),
                );
                let width = text.chars().count() as f64 * size * CHAR_EM;
                let rotated = node.attribute("transform").is_some();
                let (w, h) = if rotated {
                    (size, width)
                } else {
                    (width, size)
                };
                let left = match (node.attribute("text-anchor"), rotated) {
                    (_, true) => x - w / 2.0,
                    (Some("middle"), _) => x - w / 2.0,
                    (Some("end"), _) => x - w,
                    _ => x,
                };
                let top = if rotated { y - h / 2.0 } else { y - size * 0.8 };
                (text, left, left + w, top, top + h)
            })
            .collect()
    }

    #[test]
    fn nothing_the_picture_says_is_drawn_over_anything_else_at_any_width() {
        for width in [MIN_WIDTH, 320, 480, 800, 1600] {
            let mut full = chart(vec![
                Panel {
                    title: "Gain".into(),
                    y_label: "Voltage".into(),
                    y_unit: "V".into(),
                    y_si: true,
                    series: vec![
                        sine(300),
                        Series {
                            color: 1,
                            ..sine(300)
                        },
                    ],
                },
                Panel {
                    title: "Phase".into(),
                    y_label: "Current".into(),
                    y_unit: "A".into(),
                    y_si: true,
                    series: vec![sine(300)],
                },
            ]);
            full.width = width;
            full.banner = vec![
                "PARTIAL RUN: the simulation stopped early (timeout) and covers 12% of the range."
                    .into(),
                "MODIFIED CIRCUIT: this run simulated a modified circuit.".into(),
            ];
            full.title =
                "Transient of board.kicad_sch, run 3fa9c1d27b04-20261030T133512-7f3a".into();
            full.legend = vec![
                (0, "v(/VOUT)".into()),
                (1, "i(BT1)".into()),
                (2, "p(M1)".into()),
            ];
            full.notes = vec!["Each trace is drawn as an envelope, the lowest and highest point of every pixel column.".into()];
            let drawn = render(&full);
            let boxes = text_boxes(&drawn.svg);
            let (w, h) = (f64::from(drawn.width), f64::from(drawn.height));
            for (text, left, right, top, bottom) in &boxes {
                assert!(
                    *left >= 0.0 && *right <= w + 1.0 && *top >= 0.0 && *bottom <= h,
                    "{text:?} is outside {w}x{h} at {width}: {left} {right} {top} {bottom}"
                );
            }
            for (i, a) in boxes.iter().enumerate() {
                for b in &boxes[i + 1..] {
                    let across = a.1 < b.2 - 0.05 && b.1 < a.2 - 0.05;
                    let down = a.3 < b.4 - 0.05 && b.3 < a.4 - 0.05;
                    assert!(!(across && down), "{:?} overlaps {:?} at {width} px", a, b);
                }
            }
            // Text and boxes: the banner holds its own lines whole and nothing else, and no
            // text is drawn inside a plot frame.
            let document = Document::parse(&drawn.svg).unwrap();
            let rects: Vec<(String, f64, f64, f64, f64)> = document
                .descendants()
                .filter(|node| node.has_tag_name("rect") && node.attribute("x").is_some())
                .map(|node| {
                    let n = |name: &str| node.attribute(name).unwrap().parse::<f64>().unwrap();
                    let fill = node.attribute("fill").unwrap().to_string();
                    (
                        fill,
                        n("x"),
                        n("x") + n("width"),
                        n("y"),
                        n("y") + n("height"),
                    )
                })
                .collect();
            let banner_lines: Vec<String> = document
                .descendants()
                .filter(|node| {
                    node.has_tag_name("text") && node.attribute("fill") == Some("#4A3000")
                })
                .map(|node| node.text().unwrap_or_default().to_string())
                .collect();
            assert!(banner_lines.len() >= 2, "{banner_lines:?}");
            let banner: Vec<_> = rects.iter().filter(|r| r.0 == "#FFF3CD").collect();
            let frames: Vec<_> = rects.iter().filter(|r| r.0 == "#FFFFFF").collect();
            assert_eq!(banner.len(), 1);
            assert_eq!(frames.len(), 2);
            // The x axis label is clear of the tick labels above it.
            let last_frame = frames.iter().map(|r| r.4).fold(0.0, f64::max);
            let x_label = boxes.iter().find(|b| b.0.starts_with("Time")).unwrap();
            assert!(
                x_label.3 - last_frame >= 17.0,
                "{x_label:?} against {last_frame}"
            );
            for (text, left, right, top, bottom) in &boxes {
                let meets = |r: &&(String, f64, f64, f64, f64)| {
                    *left < r.2 && r.1 < *right && *top < r.4 && r.3 < *bottom
                };
                let inside = |r: &&(String, f64, f64, f64, f64)| {
                    *left >= r.1 && *right <= r.2 && *top >= r.3 && *bottom <= r.4
                };
                let is_banner_line = banner_lines.contains(text);
                let in_banner = banner.iter().any(inside);
                assert_eq!(in_banner, is_banner_line, "{text:?} at {width} px");
                assert!(
                    in_banner || !banner.iter().any(meets),
                    "{text:?} crosses the banner at {width} px"
                );
                assert!(
                    !frames.iter().any(meets),
                    "{text:?} is drawn inside a plot at {width} px"
                );
            }
        }
    }

    #[test]
    fn stretches_of_one_point_among_gaps_are_marks_and_a_title_fits_the_narrowest_picture() {
        // Fifty single points, more than a line a column cannot be, and more than the
        // forty a short trace gets marks for.
        let x: Vec<f64> = (0..100).map(f64::from).collect();
        let y: Vec<f64> = (0..100)
            .map(|i| if i % 2 == 0 { f64::NAN } else { f64::from(i) })
            .collect();
        let lone = render(&chart(vec![panel(vec![line(0, x, y)])]));
        assert_eq!(lone.svg.matches("<circle").count(), 50);
        // A panel title as long as a title may be, at the narrowest.
        let mut narrow = chart(vec![
            Panel {
                title: "t".repeat(64),
                ..panel(vec![sine(10)])
            },
            panel(vec![sine(10)]),
        ]);
        narrow.width = MIN_WIDTH;
        let boxes = text_boxes(&render(&narrow).svg);
        let title = boxes.iter().find(|b| b.0.starts_with("ttt")).unwrap();
        assert!(title.2 <= f64::from(MIN_WIDTH), "{title:?}");
    }

    /// Every text inside the picture, and none over another.
    fn assert_laid_out(drawn: &Rendered, what: &str) {
        let boxes = text_boxes(&drawn.svg);
        let (w, h) = (f64::from(drawn.width), f64::from(drawn.height));
        for (text, left, right, top, bottom) in &boxes {
            assert!(
                *left >= 0.0 && *right <= w + 1.0 && *top >= 0.0 && *bottom <= h,
                "{what}: {text:?} is outside {w}x{h}: {left} {right} {top} {bottom}"
            );
        }
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let across = a.1 < b.2 - 0.05 && b.1 < a.2 - 0.05;
                let down = a.3 < b.4 - 0.05 && b.3 < a.4 - 0.05;
                assert!(!(across && down), "{what}: {a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn labels_as_long_as_they_may_be_stay_in_the_picture_and_in_their_panel() {
        for width in [MIN_WIDTH, 320, 800] {
            let mut long = chart(vec![
                Panel {
                    title: "V(OUTPUT_FILTER_STAGE_2)".into(),
                    y_label: "Y".repeat(80),
                    y_unit: "V".into(),
                    y_si: true,
                    series: vec![sine(50)],
                },
                Panel {
                    title: "W".repeat(64),
                    y_label: "Z".repeat(80),
                    y_unit: "A".into(),
                    y_si: true,
                    series: vec![sine(50)],
                },
            ]);
            long.width = width;
            long.x.label = "X".repeat(80);
            long.x.unit = "Hz".into();
            long.legend = vec![(0, "L".repeat(80)), (1, "M".repeat(80))];
            assert_laid_out(&render(&long), &format!("width {width}"));
        }
    }

    #[test]
    fn noise_on_a_large_offset_reads_as_a_flat_line_with_ticks_that_tell_apart() {
        // A 5 V rail with 1e-8 of noise: without a floor on the span, six ticks
        // would read `5.000e0` alike.
        let x: Vec<f64> = (0..50).map(f64::from).collect();
        let y: Vec<f64> = (0..50).map(|i| 5.0 + 1e-8 * f64::from(i % 3)).collect();
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        let labels: Vec<String> = texts(&drawn.svg)
            .into_iter()
            .filter(|t| t.starts_with("5.") || t.starts_with("4."))
            .collect();
        assert!(labels.len() >= 3, "{labels:?}");
        let distinct: std::collections::BTreeSet<&String> = labels.iter().collect();
        assert_eq!(distinct.len(), labels.len(), "{labels:?}");
        assert!(
            labels.iter().all(|l| l.chars().count() <= MAX_TICK_CHARS),
            "{labels:?}"
        );
        // The same on the x axis: a hair of time.
        let mut hair = chart(vec![panel(vec![line(
            0,
            vec![1.0, 1.0000002],
            vec![0.0, 1.0],
        )])]);
        hair.x.si = false;
        let labels: Vec<String> = texts(&render(&hair).svg)
            .into_iter()
            .filter(|t| t.starts_with("1.0"))
            .collect();
        let distinct: std::collections::BTreeSet<&String> = labels.iter().collect();
        assert!(
            labels.len() >= 2 && distinct.len() == labels.len(),
            "{labels:?}"
        );
        // A span that is wide enough is left as it was.
        assert_eq!(widen(0.0, 1.0, false, 0.06), (-0.06, 1.06));
        let (lo, hi) = widen(5.0, 5.0 + 1e-12, false, 0.0);
        assert!(hi - lo >= 5e-6 && hi - lo < 6e-6, "{lo} {hi}");
    }

    #[test]
    fn many_stretches_that_are_each_small_still_add_up_to_a_bounded_file() {
        // 700 stretches of 1,400 points, the cut a run's own width would let through:
        // 980,000 points in one trace.
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for stretch in 0..700 {
            for point in 0..1400 {
                x.push(f64::from(stretch * 1400 + point));
                y.push(f64::from(point % 9));
            }
            x.push(f64::from(stretch * 1400 + 1400));
            y.push(f64::NAN);
        }
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        assert!(drawn.svg.len() < 400_000, "{} bytes", drawn.svg.len());
        // Fewer, longer stretches that stay apart: each cut by its own span, and all
        // under the cap together.
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for stretch in 0..200 {
            for point in 0..3000 {
                x.push(f64::from(stretch * 3000 + point));
                y.push(f64::from(point % 9));
            }
            x.push(f64::from(stretch * 3000 + 3000));
            y.push(f64::NAN);
        }
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        assert!(
            drawn.points_drawn <= 4 * 707 + 400,
            "{}",
            drawn.points_drawn
        );
        assert!(drawn.svg.len() < 200_000, "{} bytes", drawn.svg.len());
        // Stretches at the same place, over and over, are held by the cap.
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for _ in 0..150 {
            for point in 0..2000 {
                x.push(f64::from(point));
                y.push(f64::from(point % 7));
            }
            x.push(0.0);
            y.push(f64::NAN);
        }
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        assert!(
            drawn.points_drawn <= 4 * 707 + 300,
            "{}",
            drawn.points_drawn
        );
    }

    #[test]
    fn a_narrow_spike_in_every_one_of_many_stretches_survives_the_cut() {
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for stretch in 0..700 {
            for point in 0..1400 {
                x.push(f64::from(stretch * 1400 + point));
                y.push(if point == 701 { 10.0 } else { 0.0 });
            }
            x.push(f64::from(stretch * 1400 + 1400));
            y.push(f64::NAN);
        }
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        let document = Document::parse(&drawn.svg).unwrap();
        let ys: Vec<f64> = document
            .descendants()
            .filter(|n| n.has_tag_name("polyline"))
            .flat_map(|n| n.attribute("points").unwrap().split(' '))
            .map(|p| p.split(',').nth(1).unwrap().parse().unwrap())
            .collect();
        let top = ys.iter().cloned().fold(f64::INFINITY, f64::min);
        let spikes = ys.iter().filter(|y| (**y - top).abs() < 0.2).count();
        assert!(spikes >= 600, "{spikes} of 700 spikes kept");
    }

    #[test]
    fn sweeps_that_share_an_x_range_keep_their_spikes_when_the_trace_is_cut_to_fit() {
        // Five curves over the same 20,000 samples, each with one narrow spike, apart by
        // NaN: together far over four points a column.
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for sweep in 0..5 {
            for point in 0..20_000 {
                x.push(f64::from(point));
                y.push(if point == 12_345 {
                    10.0 + f64::from(sweep)
                } else {
                    0.0
                });
            }
            x.push(0.0);
            y.push(f64::NAN);
        }
        let drawn = render(&chart(vec![panel(vec![line(0, x, y)])]));
        assert!(drawn.points_drawn <= 4 * 707, "{}", drawn.points_drawn);
        let document = Document::parse(&drawn.svg).unwrap();
        let lines: Vec<Vec<f64>> = document
            .descendants()
            .filter(|n| n.has_tag_name("polyline"))
            .map(|n| {
                n.attribute("points")
                    .unwrap()
                    .split(' ')
                    .map(|p| p.split(',').nth(1).unwrap().parse().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(lines.len(), 5);
        // The pixel row of the highest spike is the least y of any, and every
        // curve's own spike stands out of its flat line by a good margin.
        for (sweep, ys) in lines.iter().enumerate() {
            let top = ys.iter().cloned().fold(f64::INFINITY, f64::min);
            let flat = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            assert!(
                flat - top > 20.0,
                "curve {sweep} lost its spike: {top} {flat}"
            );
        }
    }

    #[test]
    fn negative_values_near_the_span_floor_still_have_ticks_that_tell_apart() {
        let y = vec![-3.3, -3.300004, -3.3, -3.300004];
        let drawn = render(&chart(vec![panel(vec![line(
            0,
            vec![0.0, 1.0, 2.0, 3.0],
            y,
        )])]));
        let labels: Vec<String> = texts(&drawn.svg)
            .into_iter()
            .filter(|t| t.starts_with("-3."))
            .collect();
        let distinct: std::collections::BTreeSet<&String> = labels.iter().collect();
        assert!(
            labels.len() >= 3 && distinct.len() == labels.len(),
            "{labels:?}"
        );
    }

    #[test]
    fn nine_digit_x_labels_on_a_hair_of_a_span_keep_clear_of_each_other_and_the_edge() {
        for width in [MIN_WIDTH, 260, 320, 800] {
            let mut hair = chart(vec![panel(vec![line(
                0,
                vec![1.2490828, 1.2490828 + 1e-9],
                vec![0.0, 1.0],
            )])]);
            hair.width = width;
            hair.x.unit = "s".into();
            assert_laid_out(&render(&hair), &format!("width {width}"));
        }
    }

    #[test]
    fn a_label_up_a_panel_is_no_taller_than_the_panel() {
        let mut long = chart(vec![
            Panel {
                y_label: "Y".repeat(80),
                y_unit: "uV".into(),
                ..panel(vec![sine(20)])
            },
            Panel {
                y_label: "Z".repeat(80),
                ..panel(vec![sine(20)])
            },
        ]);
        long.width = 800;
        let two = text_boxes(&render(&long).svg);
        // Rotated texts are as wide as their font is high.
        let rotated: Vec<_> = two
            .iter()
            .filter(|b| (b.2 - b.1 - 12.0).abs() < 0.01)
            .collect();
        assert_eq!(rotated.len(), 2);
        for label in rotated {
            assert!(label.4 - label.3 <= PANEL_HEIGHT, "{label:?}");
        }
        long.panels.truncate(1);
        let one = text_boxes(&render(&long).svg);
        let label = one
            .iter()
            .find(|b| (b.2 - b.1 - 12.0).abs() < 0.01)
            .unwrap();
        assert!(label.4 - label.3 <= SINGLE_PANEL_HEIGHT, "{label:?}");
    }
}
