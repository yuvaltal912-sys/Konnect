//! `get_schematic_netlist` and `compare_netlist` — what the drawn circuit connects.
//!
//! The netlist comes from KiCad itself: the saved sheets are copied into a
//! private folder and `kicad-cli sch export netlist` runs on the copy, so the
//! answer covers the whole hierarchy with KiCad's own rules for labels, power
//! symbols, sheet pins and buses, and the live files are never opened. The
//! per-file net graph in `sch_connectivity` cannot do this: it does not join
//! nets across sheets.
//!
//! Connectivity is compared as a partition of pins, never by net name. A name
//! can be KiCad's own (`Net-(R1-Pad2)`, `/Child/SIG`) or the caller's, and two
//! circuits that join the same pins are the same circuit under any names.

use crate::mcp::error::ToolErrorKind;
use crate::mcp::protocol::CallToolResult;
use crate::simulation::kicad_netlist::{self, KiCadNetlist};
use crate::simulation::snapshot;
use crate::tool;
use crate::tools::{cli, invalid_arg, require_str, sch_export, ToolContext, ToolDef};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Longest `kicad-cli` is given to export one private copy.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(60);
/// Nets per page unless the caller asks otherwise, and the most it may ask for.
const DEFAULT_NET_LIMIT: usize = 200;
const MAX_NET_LIMIT: usize = 1000;
/// Characters of net data in one response. The client rejects results near
/// 50,000 characters, so a page stops before that and says where to resume.
const NET_BYTE_BUDGET: usize = 40_000;
/// Pins listed for one net. A ground rail can hold hundreds.
const MAX_PINS_PER_NET: usize = 300;
/// Findings listed per category unless the caller asks otherwise.
const DEFAULT_FINDING_LIMIT: usize = 25;
const MAX_FINDING_LIMIT: usize = 500;
/// Pins listed inside one finding.
const MAX_PINS_PER_FINDING: usize = 40;
/// Largest reference netlist file read.
const MAX_NETLIST_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "get_schematic_netlist",
            "Read the connectivity of a whole schematic hierarchy as \
             {net_name: [\"REF.PIN\", ...]}, from KiCad's own netlist export of a \
             private copy of the saved sheets (the live files are never opened, and \
             unsaved editor changes are not seen). Nets KiCad names itself, such as \
             Net-(R1-Pad2) and unconnected-(U1-Pad3), are included; power symbols \
             and hierarchical names are resolved across sheets. Pass the root sheet \
             or the .kicad_pro. Results are paged: `limit` and `offset` choose the \
             nets, `truncated` says more remain, and `next_offset` resumes. Use \
             compare_netlist to check the result against intent.",
            json!({
                "type": "object",
                "properties": {
                    "schematic": {
                        "type": "string",
                        "description": "Root .kicad_sch of the project, or its .kicad_pro"
                    },
                    "limit": {
                        "type": "integer", "minimum": 1, "maximum": MAX_NET_LIMIT,
                        "default": DEFAULT_NET_LIMIT,
                        "description": "Most nets to return (a page also stops at a size cap)"
                    },
                    "offset": {
                        "type": "integer", "minimum": 0, "default": 0,
                        "description": "Nets to skip, in name order"
                    },
                    "name_contains": {
                        "type": "string",
                        "description": "Only nets whose name contains this text (case-sensitive)"
                    }
                },
                "required": ["schematic"]
            }),
            |args, ctx| async move { handle_get_netlist(args, ctx).await }
        ),
        tool!(
            "compare_netlist",
            "Check that the drawn circuit connects what was intended, by pin sets \
             and not by net name. Give `schematic` (the root sheet or .kicad_pro) \
             and exactly one of: `intended` ({net: [\"REF.PIN\", ...]}), \
             `reference_schematic` (a second project) or `reference_netlist` (a \
             KiCad .net file). Reports `missing` (an intended pin that is not in \
             the design), `split` (one intended net spread over several nets, so \
             connections are missing), `merged` (one net holding pins of several \
             intended nets, so connections are extra) and `extra` (pins joined to \
             a net that the intent does not give them), then a verdict: pass, \
             fail, or incomplete when pins could not be told apart. With \
             `intended`, scope `listed_pins` (default) looks only at the pins you \
             list, so a partial intent is fine; `complete` also flags every other \
             pin joined to them. The two-design forms always compare completely. \
             Names never matter, so unnamed and hierarchical nets need no \
             normalising.",
            json!({
                "type": "object",
                "properties": {
                    "schematic": {
                        "type": "string",
                        "description": "Root .kicad_sch of the design to check, or its .kicad_pro"
                    },
                    "intended": {
                        "type": "object",
                        "description": "Intended connectivity: net name -> pins as \"REF.PIN\"",
                        "additionalProperties": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "reference_schematic": {
                        "type": "string",
                        "description": "Root .kicad_sch (or .kicad_pro) of a second design to compare against"
                    },
                    "reference_netlist": {
                        "type": "string",
                        "description": "Path of a KiCad-format netlist (.net, kicadsexpr) to compare against"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["listed_pins", "complete"],
                        "default": "listed_pins",
                        "description": "With `intended` only: whether pins the intent does not list are ignored or checked"
                    },
                    "limit": {
                        "type": "integer", "minimum": 1, "maximum": MAX_FINDING_LIMIT,
                        "default": DEFAULT_FINDING_LIMIT,
                        "description": "Findings listed per category"
                    }
                },
                "required": ["schematic"]
            }),
            |args, ctx| async move { handle_compare_netlist(args, ctx).await }
        ),
    ]
}

// ─── Connectivity ────────────────────────────────────────────────────────────

/// A design's nets as sets of `REF.PIN` ids.
#[derive(Debug, Default)]
struct Connectivity {
    nets: BTreeMap<String, Vec<String>>,
    pin_net: HashMap<String, String>,
    /// Pins the export lists on more than one net, with those nets. A duplicate
    /// reference (two `R1`) does this, and it makes `R1.2` mean two things.
    ambiguous: BTreeMap<String, Vec<String>>,
    components: usize,
    unannotated: Vec<String>,
}

fn pin_id(reference: &str, pin: &str) -> String {
    format!("{reference}.{pin}")
}

fn connectivity(netlist: &KiCadNetlist) -> Connectivity {
    let mut out = Connectivity {
        components: netlist.components.len(),
        ..Connectivity::default()
    };
    let mut seen_nets: HashMap<String, BTreeSet<String>> = HashMap::new();
    for net in &netlist.nets {
        let pins = out.nets.entry(net.name.clone()).or_default();
        for pin in &net.pins {
            let id = pin_id(&pin.reference, &pin.pin);
            let nets_of_pin = seen_nets.entry(id.clone()).or_default();
            if nets_of_pin.insert(net.name.clone()) {
                pins.push(id.clone());
            }
            out.pin_net.entry(id).or_insert_with(|| net.name.clone());
        }
    }
    for (id, nets) in seen_nets {
        if nets.len() > 1 {
            out.ambiguous.insert(id, nets.into_iter().collect());
        }
    }
    for pins in out.nets.values_mut() {
        pins.sort_by(|a, b| natural_cmp(a, b));
        pins.dedup();
    }
    let mut unannotated: BTreeSet<String> = netlist
        .components
        .iter()
        .filter(|component| component.reference.contains('?'))
        .map(|component| component.reference.clone())
        .collect();
    out.unannotated = std::mem::take(&mut unannotated).into_iter().collect();
    out
}

/// Order `R2` before `R10` and pin `2` before pin `10`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        chars.next();
                    }
                    digits
                };
                let (left, right) = (run(&mut a), run(&mut b));
                let (left_trim, right_trim) =
                    (left.trim_start_matches('0'), right.trim_start_matches('0'));
                let order = left_trim
                    .len()
                    .cmp(&right_trim.len())
                    .then_with(|| left_trim.cmp(right_trim))
                    .then_with(|| left.len().cmp(&right.len()));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

// ─── Comparison ──────────────────────────────────────────────────────────────

/// Which pins a comparison looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Only the pins the intent lists.
    ListedPins,
    /// Every pin: one the intent does not list but the design joins to a
    /// listed pin, or to another unlisted one, is a finding.
    Complete,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::ListedPins => "listed_pins",
            Scope::Complete => "complete",
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Comparison {
    /// `(intended net, pin)` for a pin the design does not have.
    missing: Vec<(String, String)>,
    /// An intended net that the design spreads over several nets.
    split: Vec<Split>,
    /// A design net that holds pins of several intended nets.
    merged: Vec<Merged>,
    /// Pins joined to a net the intent does not give them.
    extra: Vec<Extra>,
}

#[derive(Debug, PartialEq, Eq)]
struct Split {
    intended_net: String,
    /// `(design net, intended pins found on it)`.
    groups: Vec<(String, Vec<String>)>,
}

#[derive(Debug, PartialEq, Eq)]
struct Merged {
    design_net: String,
    /// `(intended net, its pins on this design net)`.
    joins: Vec<(String, Vec<String>)>,
}

#[derive(Debug, PartialEq, Eq)]
struct Extra {
    design_net: String,
    /// The intended nets whose pins sit on `design_net`; empty when none does.
    intended_nets: Vec<String>,
    pins: Vec<String>,
}

impl Comparison {
    fn is_empty(&self) -> bool {
        self.missing.is_empty()
            && self.split.is_empty()
            && self.merged.is_empty()
            && self.extra.is_empty()
    }
}

fn compare(
    design: &Connectivity,
    intended: &BTreeMap<String, BTreeSet<String>>,
    scope: Scope,
) -> Comparison {
    let intended_net_of: HashMap<&str, &str> = intended
        .iter()
        .flat_map(|(net, pins)| pins.iter().map(move |pin| (pin.as_str(), net.as_str())))
        .collect();
    let mut out = Comparison::default();

    for (net, pins) in intended {
        let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for pin in pins {
            match design.pin_net.get(pin) {
                Some(design_net) => groups.entry(design_net).or_default().push(pin.clone()),
                None => out.missing.push((net.clone(), pin.clone())),
            }
        }
        if groups.len() > 1 {
            out.split.push(Split {
                intended_net: net.clone(),
                groups: sorted_groups(groups),
            });
        }
    }

    for (design_net, pins) in &design.nets {
        let mut joins: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        let mut unlisted = Vec::new();
        for pin in pins {
            match intended_net_of.get(pin.as_str()) {
                Some(intended_net) => joins.entry(intended_net).or_default().push(pin.clone()),
                None => unlisted.push(pin.clone()),
            }
        }
        let intended_nets: Vec<String> = joins.keys().map(|net| (*net).to_owned()).collect();
        if joins.len() > 1 {
            out.merged.push(Merged {
                design_net: design_net.clone(),
                joins: sorted_groups(joins),
            });
        }
        // A pin alone on its net joins nothing, so it is never a connection.
        if scope == Scope::Complete && !unlisted.is_empty() && pins.len() > 1 {
            out.extra.push(Extra {
                design_net: design_net.clone(),
                intended_nets,
                pins: unlisted,
            });
        }
    }
    out
}

fn sorted_groups(groups: BTreeMap<&str, Vec<String>>) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = groups
        .into_iter()
        .map(|(net, mut pins)| {
            pins.sort_by(|a, b| natural_cmp(a, b));
            (net.to_owned(), pins)
        })
        .collect();
    // The biggest group first: it is the one the caller most likely meant.
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    groups
}

/// Reasons a clean comparison cannot be trusted. Empty means it can.
fn ambiguity(
    design: &Connectivity,
    intended: &BTreeMap<String, BTreeSet<String>>,
    scope: Scope,
) -> Vec<String> {
    let relevant =
        |pin: &str| scope == Scope::Complete || intended.values().any(|pins| pins.contains(pin));
    design
        .ambiguous
        .iter()
        .filter(|(pin, _)| relevant(pin))
        .map(|(pin, nets)| {
            format!(
                "{pin} is listed on {} nets ({}): a reference is used more than once, so the \
                 pin cannot be told apart. Annotate the schematic so every reference is unique.",
                nets.len(),
                nets.join(", ")
            )
        })
        .collect()
}

// ─── Loading ─────────────────────────────────────────────────────────────────

struct Loaded {
    root: PathBuf,
    netlist: KiCadNetlist,
    locks_present: Vec<PathBuf>,
}

/// Resolve the root sheet from a `.kicad_sch` or `.kicad_pro` path and refuse a
/// sub-sheet, which KiCad would export as if it were the whole design.
fn resolve_root(value: &str, field: &str) -> Result<PathBuf, CallToolResult> {
    let path = PathBuf::from(value);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let schematic = match extension.as_deref() {
        Some("kicad_sch") => path,
        Some("kicad_pro") => path.with_extension("kicad_sch"),
        _ => {
            return Err(invalid_arg(
                field,
                "must be a .kicad_sch root sheet or a .kicad_pro project file",
            ))
        }
    };
    if !schematic.is_file() {
        return Err(CallToolResult::error_kind(
            ToolErrorKind::FileNotFound {
                path: schematic.display().to_string(),
            },
            format!("Schematic not found: {}", schematic.display()),
        ));
    }
    match sch_export::owning_project_root(&schematic) {
        Ok(None) => Ok(schematic),
        Ok(Some(root)) => {
            let reason = format!(
                "{} is a sheet inside the project rooted at {}. Exporting it alone would read \
                 only part of the design: use {}.",
                schematic.display(),
                root.display(),
                root.display()
            );
            Err(CallToolResult::error_kind(
                ToolErrorKind::InvalidArgument {
                    field: field.to_string(),
                    reason: reason.clone(),
                },
                reason,
            ))
        }
        Err(error) => Err(error.into_tool_result()),
    }
}

/// Copy the saved hierarchy into a private folder and export its netlist there.
async fn load_schematic(kicad_cli: &str, schematic: &Path) -> anyhow::Result<Loaded> {
    let scratch = tempfile::Builder::new()
        .prefix("konnect-netlist-")
        .tempdir()?;
    let source = scratch.path().join("src");
    std::fs::create_dir(&source)?;
    let schematic = schematic.canonicalize()?;
    let snapshot = snapshot::take_snapshot(&schematic, &source)?;
    let text = cli::export_kicad_netlist_text(
        kicad_cli,
        &snapshot.root_sheet,
        scratch.path(),
        EXPORT_TIMEOUT,
    )
    .await?;
    let netlist = kicad_netlist::parse(&text)?;
    Ok(Loaded {
        root: schematic,
        netlist,
        locks_present: snapshot.locks_present,
    })
}

fn source_block(loaded: &Loaded) -> Value {
    let mut source = json!({
        "schematic": loaded.root.display().to_string(),
        "origin": "kicad-cli sch export netlist on a private copy of the saved sheets",
        "saved_files_only": true,
    });
    if !loaded.locks_present.is_empty() {
        source["editor_locks"] = json!(loaded
            .locks_present
            .iter()
            .map(|lock| lock.display().to_string())
            .collect::<Vec<_>>());
        source["warning"] = json!(
            "KiCad has this project open. Changes not yet saved in the editor are not in this result."
        );
    }
    source
}

// ─── get_schematic_netlist ───────────────────────────────────────────────────

fn bounded_usize(
    args: &Value,
    field: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, CallToolResult> {
    match &args[field] {
        Value::Null => Ok(default),
        value => {
            let number = value
                .as_u64()
                .filter(|_| value.is_u64())
                .ok_or_else(|| invalid_arg(field, "must be a non-negative integer"))?;
            let number =
                usize::try_from(number).map_err(|_| invalid_arg(field, "is out of range"))?;
            if number < min || number > max {
                return Err(invalid_arg(
                    field,
                    &format!("must be between {min} and {max}"),
                ));
            }
            Ok(number)
        }
    }
}

async fn handle_get_netlist(args: &Value, ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    let schematic = match require_str(args, "schematic") {
        Ok(value) => value,
        Err(refusal) => return Ok(refusal),
    };
    let limit = match bounded_usize(args, "limit", DEFAULT_NET_LIMIT, 1, MAX_NET_LIMIT) {
        Ok(value) => value,
        Err(refusal) => return Ok(refusal),
    };
    let offset = match bounded_usize(args, "offset", 0, 0, usize::MAX) {
        Ok(value) => value,
        Err(refusal) => return Ok(refusal),
    };
    let name_contains = match &args["name_contains"] {
        Value::Null => None,
        Value::String(text) => Some(text.as_str()),
        _ => return Ok(invalid_arg("name_contains", "must be a string")),
    };
    let root = match resolve_root(schematic, "schematic") {
        Ok(root) => root,
        Err(refusal) => return Ok(refusal),
    };
    let loaded = load_schematic(&ctx.config.kicad_cli, &root).await?;
    let design = connectivity(&loaded.netlist);
    Ok(CallToolResult::json(&netlist_response(
        &design,
        &loaded,
        limit,
        offset,
        name_contains,
    )))
}

fn netlist_response(
    design: &Connectivity,
    loaded: &Loaded,
    limit: usize,
    offset: usize,
    name_contains: Option<&str>,
) -> Value {
    let matching: Vec<(&String, &Vec<String>)> = design
        .nets
        .iter()
        .filter(|(name, _)| name_contains.is_none_or(|text| name.contains(text)))
        .collect();
    let mut nets = Map::new();
    let mut shortened = Map::new();
    let mut used = 0usize;
    let mut taken = 0usize;
    for (name, pins) in matching.iter().skip(offset).take(limit) {
        let shown = &pins[..pins.len().min(MAX_PINS_PER_NET)];
        let cost = name.len() + shown.iter().map(|pin| pin.len() + 4).sum::<usize>() + 8;
        // A page always holds at least one net, so an enormous one cannot stall paging.
        if taken > 0 && used + cost > NET_BYTE_BUDGET {
            break;
        }
        used += cost;
        taken += 1;
        nets.insert((*name).clone(), json!(shown));
        if shown.len() < pins.len() {
            shortened.insert((*name).clone(), json!(pins.len()));
        }
    }
    let next = offset + taken;
    let truncated = next < matching.len();
    let mut response = json!({
        "source": source_block(loaded),
        "net_count": design.nets.len(),
        "matching_nets": matching.len(),
        "component_count": design.components,
        "pin_count": design.pin_net.len(),
        "offset": offset,
        "limit": limit,
        "returned": taken,
        "truncated": truncated,
        "nets": nets,
    });
    if truncated {
        response["next_offset"] = json!(next);
    }
    if !shortened.is_empty() {
        response["pins_truncated"] = json!({
            "note": format!("These nets list only their first {MAX_PINS_PER_NET} pins; the value is the full pin count."),
            "nets": shortened,
        });
    }
    let mut warnings = Vec::new();
    if !design.ambiguous.is_empty() {
        let pins: Vec<&String> = design.ambiguous.keys().collect();
        warnings.push(json!({
            "code": "ambiguous_pins",
            "message": "These pins appear on more than one net because a reference is used more than once. Annotate the schematic so every reference is unique.",
            "count": pins.len(),
            "pins": pins.into_iter().take(MAX_PINS_PER_FINDING).collect::<Vec<_>>(),
        }));
    }
    if !design.unannotated.is_empty() {
        warnings.push(json!({
            "code": "unannotated_references",
            "message": "These references are not annotated yet.",
            "references": design.unannotated.iter().take(MAX_PINS_PER_FINDING).collect::<Vec<_>>(),
        }));
    }
    if !warnings.is_empty() {
        response["warnings"] = json!(warnings);
    }
    response
}

// ─── compare_netlist ─────────────────────────────────────────────────────────

/// Validate `{net: ["REF.PIN", ...]}` into pin sets.
fn parse_intended(value: &Value) -> Result<BTreeMap<String, BTreeSet<String>>, CallToolResult> {
    let Some(object) = value.as_object() else {
        return Err(invalid_arg(
            "intended",
            "must be an object of net name to a list of \"REF.PIN\" strings",
        ));
    };
    if object.is_empty() {
        return Err(invalid_arg("intended", "lists no nets"));
    }
    let mut nets = BTreeMap::new();
    let mut owner: HashMap<String, String> = HashMap::new();
    for (net, pins) in object {
        let Some(pins) = pins.as_array() else {
            return Err(invalid_arg(
                "intended",
                &format!("net {net:?} must be a list of \"REF.PIN\" strings"),
            ));
        };
        if pins.is_empty() {
            return Err(invalid_arg(
                "intended",
                &format!("net {net:?} lists no pins"),
            ));
        }
        let mut set = BTreeSet::new();
        for pin in pins {
            let Some(text) = pin.as_str().map(str::trim) else {
                return Err(invalid_arg(
                    "intended",
                    &format!("net {net:?} holds a value that is not a string"),
                ));
            };
            match text.split_once('.') {
                Some((reference, number)) if !reference.is_empty() && !number.is_empty() => {}
                _ => {
                    return Err(invalid_arg(
                        "intended",
                        &format!("{text:?} in net {net:?} is not of the form \"REF.PIN\""),
                    ))
                }
            }
            if let Some(other) = owner.insert(text.to_owned(), net.clone()) {
                if other != *net {
                    return Err(invalid_arg(
                        "intended",
                        &format!("pin {text} is listed in both net {other:?} and net {net:?}"),
                    ));
                }
            }
            set.insert(text.to_owned());
        }
        nets.insert(net.clone(), set);
    }
    Ok(nets)
}

fn read_netlist_file(value: &str) -> Result<KiCadNetlist, CallToolResult> {
    let path = PathBuf::from(value);
    let size = std::fs::metadata(&path).map_err(|_| {
        CallToolResult::error_kind(
            ToolErrorKind::FileNotFound {
                path: path.display().to_string(),
            },
            format!("Netlist not found: {}", path.display()),
        )
    })?;
    if !size.is_file() || size.len() > MAX_NETLIST_FILE_BYTES {
        return Err(invalid_arg(
            "reference_netlist",
            "must be a netlist file of at most 64 MiB",
        ));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| invalid_arg("reference_netlist", &format!("cannot be read: {error}")))?;
    kicad_netlist::parse(&text).map_err(|error| {
        invalid_arg(
            "reference_netlist",
            &format!("is not a KiCad-format netlist (kicad-cli --format kicadsexpr): {error:#}"),
        )
    })
}

/// What the design is held to.
struct Reference {
    intended: BTreeMap<String, BTreeSet<String>>,
    scope: Scope,
    /// Where it came from, for the response.
    description: Value,
    /// Reference-side problems that stop a clean pass.
    problems: Vec<String>,
}

fn reference_from_design(design: &Connectivity, description: Value) -> Reference {
    let problems = design
        .ambiguous
        .iter()
        .map(|(pin, nets)| {
            format!(
                "reference: {pin} is listed on {} nets ({}), so it cannot be told apart",
                nets.len(),
                nets.join(", ")
            )
        })
        .collect();
    Reference {
        intended: design
            .nets
            .iter()
            .map(|(name, pins)| (name.clone(), pins.iter().cloned().collect()))
            .collect(),
        scope: Scope::Complete,
        description,
        problems,
    }
}

async fn handle_compare_netlist(args: &Value, ctx: &ToolContext) -> anyhow::Result<CallToolResult> {
    let schematic = match require_str(args, "schematic") {
        Ok(value) => value,
        Err(refusal) => return Ok(refusal),
    };
    let limit = match bounded_usize(args, "limit", DEFAULT_FINDING_LIMIT, 1, MAX_FINDING_LIMIT) {
        Ok(value) => value,
        Err(refusal) => return Ok(refusal),
    };
    let given: Vec<&str> = ["intended", "reference_schematic", "reference_netlist"]
        .into_iter()
        .filter(|field| !args[*field].is_null())
        .collect();
    if given.len() != 1 {
        return Ok(invalid_arg(
            "intended",
            "give exactly one of `intended`, `reference_schematic` and `reference_netlist`",
        ));
    }
    let scope = match args["scope"].as_str() {
        None if args["scope"].is_null() => None,
        Some("listed_pins") => Some(Scope::ListedPins),
        Some("complete") => Some(Scope::Complete),
        _ => {
            return Ok(invalid_arg(
                "scope",
                "must be \"listed_pins\" or \"complete\"",
            ))
        }
    };
    if given[0] != "intended" && scope == Some(Scope::ListedPins) {
        return Ok(invalid_arg(
            "scope",
            "`listed_pins` applies only to `intended`; a second design is always compared completely",
        ));
    }

    // Everything that can be refused is refused before kicad-cli runs.
    let root = match resolve_root(schematic, "schematic") {
        Ok(root) => root,
        Err(refusal) => return Ok(refusal),
    };
    let mut reference = match given[0] {
        "intended" => match parse_intended(&args["intended"]) {
            Ok(intended) => Reference {
                intended,
                scope: scope.unwrap_or(Scope::ListedPins),
                description: json!({ "kind": "intended" }),
                problems: Vec::new(),
            },
            Err(refusal) => return Ok(refusal),
        },
        "reference_netlist" => {
            let path = args["reference_netlist"].as_str().unwrap_or_default();
            match args["reference_netlist"].as_str() {
                Some(_) => {}
                None => return Ok(invalid_arg("reference_netlist", "must be a string")),
            }
            match read_netlist_file(path) {
                Ok(netlist) => reference_from_design(
                    &connectivity(&netlist),
                    json!({ "kind": "reference_netlist", "netlist": path }),
                ),
                Err(refusal) => return Ok(refusal),
            }
        }
        _ => {
            let Some(path) = args["reference_schematic"].as_str() else {
                return Ok(invalid_arg("reference_schematic", "must be a string"));
            };
            let reference_root = match resolve_root(path, "reference_schematic") {
                Ok(root) => root,
                Err(refusal) => return Ok(refusal),
            };
            let loaded = load_schematic(&ctx.config.kicad_cli, &reference_root).await?;
            let mut reference = reference_from_design(
                &connectivity(&loaded.netlist),
                json!({ "kind": "reference_schematic", "source": source_block(&loaded) }),
            );
            reference.scope = Scope::Complete;
            reference
        }
    };

    let loaded = load_schematic(&ctx.config.kicad_cli, &root).await?;
    let design = connectivity(&loaded.netlist);
    let comparison = compare(&design, &reference.intended, reference.scope);
    let mut problems = ambiguity(&design, &reference.intended, reference.scope);
    problems.append(&mut reference.problems);
    Ok(CallToolResult::json(&compare_response(
        &design,
        &loaded,
        &reference,
        &comparison,
        &problems,
        limit,
    )))
}

fn compare_response(
    design: &Connectivity,
    loaded: &Loaded,
    reference: &Reference,
    comparison: &Comparison,
    problems: &[String],
    limit: usize,
) -> Value {
    let (result, passed) = if !comparison.is_empty() {
        ("fail", false)
    } else if !problems.is_empty() {
        ("incomplete", false)
    } else {
        ("pass", true)
    };
    let counts = json!({
        "missing": comparison.missing.len(),
        "split": comparison.split.len(),
        "merged": comparison.merged.len(),
        "extra": comparison.extra.len(),
    });
    let truncated = [
        comparison.missing.len(),
        comparison.split.len(),
        comparison.merged.len(),
        comparison.extra.len(),
    ]
    .into_iter()
    .any(|count| count > limit);

    let missing: Vec<Value> = comparison
        .missing
        .iter()
        .take(limit)
        .map(|(net, pin)| json!({ "intended_net": net, "pin": pin }))
        .collect();
    let split: Vec<Value> = comparison
        .split
        .iter()
        .take(limit)
        .map(|finding| {
            json!({
                "intended_net": finding.intended_net,
                "found_on": group_list(&finding.groups, "design_net"),
            })
        })
        .collect();
    let merged: Vec<Value> = comparison
        .merged
        .iter()
        .take(limit)
        .map(|finding| {
            json!({
                "design_net": finding.design_net,
                "joins": group_list(&finding.joins, "intended_net"),
            })
        })
        .collect();
    let extra: Vec<Value> = comparison
        .extra
        .iter()
        .take(limit)
        .map(|finding| {
            let mut entry = json!({
                "design_net": finding.design_net,
                "intended_nets": finding.intended_nets,
            });
            insert_pins(&mut entry, "pins", &finding.pins);
            entry
        })
        .collect();

    let listed_pins: usize = reference.intended.values().map(BTreeSet::len).sum();
    let mut response = json!({
        "result": result,
        "passed": passed,
        "counts": counts,
        "missing": missing,
        "split": split,
        "merged": merged,
        "extra": extra,
        "truncated": truncated,
        "limit": limit,
        "compared": {
            "scope": reference.scope.as_str(),
            "against": reference.description,
            "reference_nets": reference.intended.len(),
            "reference_pins": listed_pins,
            "design_nets": design.nets.len(),
            "design_pins": design.pin_net.len(),
            "design_components": design.components,
        },
        "source": source_block(loaded),
    });
    if !problems.is_empty() {
        response["incomplete_because"] = json!(problems
            .iter()
            .take(MAX_PINS_PER_FINDING)
            .collect::<Vec<_>>());
    }
    if !design.unannotated.is_empty() {
        response["warnings"] = json!([{
            "code": "unannotated_references",
            "message": "These references are not annotated yet, so their pins cannot be named reliably.",
            "references": design.unannotated.iter().take(MAX_PINS_PER_FINDING).collect::<Vec<_>>(),
        }]);
    }
    response
}

fn group_list(groups: &[(String, Vec<String>)], key: &str) -> Vec<Value> {
    groups
        .iter()
        .map(|(net, pins)| {
            let mut entry = Map::new();
            entry.insert(key.to_owned(), json!(net));
            let mut value = Value::Object(entry);
            insert_pins(&mut value, "pins", pins);
            value
        })
        .collect()
}

fn insert_pins(target: &mut Value, key: &str, pins: &[String]) {
    target[key] = json!(&pins[..pins.len().min(MAX_PINS_PER_FINDING)]);
    if pins.len() > MAX_PINS_PER_FINDING {
        target["pins_total"] = json!(pins.len());
    }
}

#[cfg(test)]
#[path = "sch_netlist_tests.rs"]
mod tests;
