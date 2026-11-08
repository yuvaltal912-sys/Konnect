//! Fail-closed KiCad PCB snapshot to Specctra DSN lowering.
//!
//! The input is the exact string returned by KiCad's IPC
//! `SaveDocumentToString`; this module never reads or edits the live board
//! file. The first supported profile is intentionally narrow and rejects
//! every feature it cannot represent without approximation.

use anyhow::{bail, Context, Result};
use konnect_ipc::IpcEffectiveRoutingRules;
use konnect_sexp::{parse_sexp, SexpNode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specctra::read::{ListTokenizer, ReadDsn};
use specctra::structure as dsn;
use specctra::write::{ListWriter, WriteSes};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Cursor};
use std::path::Path;

const UM_PER_MM: f64 = 1_000.0;
const DSN_RESOLUTION: f32 = 10.0;
/// The most an arc or circle may stray from its chord approximation, in mm.
/// Specctra has no arc in the shapes the DSN writer offers, so board outlines,
/// circles and zone-outline arcs are written as chords. 5 um is far under the
/// rounding of the micrometre grid the DSN is written on, and under any edge
/// clearance a fab allows.
const ARC_TOLERANCE_MM: f64 = 0.005;
/// The most chords one arc or circle may be cut into.
const MAX_ARC_SEGMENTS: usize = 4096;

#[derive(Debug, Clone)]
pub(crate) struct ExportBundle {
    pub dsn: String,
    pub manifest: String,
    pub source_sha256: String,
    pub component_count: usize,
    pub pad_count: usize,
    pub net_count: usize,
    pub class_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PadShape {
    Circle,
    Rect,
    /// Written as its bounding rectangle, which is never smaller than the pad.
    #[serde(rename = "roundrect")]
    RoundRect,
    /// Written as a stadium: a path of the short dimension's width.
    Oval,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PadstackKey {
    shape: PadShape,
    layers: Vec<String>,
    size_x_um: i64,
    size_y_um: i64,
    drill_um: Option<i64>,
    /// Where the copper sits relative to the hole, in the pad's own frame
    /// with Y up: KiCad's `(drill … (offset x y))`.
    offset_x_um: i64,
    offset_y_um: i64,
}

#[derive(Debug, Clone)]
struct PadModel {
    /// KiCad's pad number, possibly empty or repeated.
    number: String,
    /// The unique DSN pin id (see [`pin_ids`]).
    dsn_id: String,
    x_um: i64,
    y_um: i64,
    rotation_degrees: f64,
    net: Option<String>,
    padstack: PadstackKey,
}

#[derive(Debug, Clone)]
struct FootprintModel {
    reference: String,
    kiid: String,
    image_name: String,
    x_um: i64,
    y_um: i64,
    rotation_degrees: f64,
    /// The KiCad side, `front` or `back`. The DSN always places a part on
    /// the front: a back part's file already holds its pads mirrored, so its
    /// image is written with every pad at its true board position and its
    /// SMD pads on B.Cu, and the router never mirrors anything.
    kicad_side: &'static str,
    pads: Vec<PadModel>,
    /// Unplated holes, written as image keepouts on every copper layer.
    holes: Vec<HoleModel>,
    /// Copper graphics and visible copper text, as image keepout shapes in
    /// footprint space.
    copper_keepouts: Vec<dsn::Shape>,
}

/// An unplated hole in image coordinates.
#[derive(Debug, Clone)]
struct HoleModel {
    x_um: i64,
    y_um: i64,
    rotation_degrees: f64,
    size_x_um: i64,
    size_y_um: i64,
}

#[derive(Debug, Clone)]
struct LockedTrackModel {
    start_x_um: i64,
    start_y_um: i64,
    end_x_um: i64,
    end_y_um: i64,
    width_um: i64,
    layer: String,
    net: String,
}

#[derive(Debug, Clone)]
struct LockedViaModel {
    x_um: i64,
    y_um: i64,
    diameter_um: i64,
    drill_um: i64,
    net: String,
}

/// What a copper zone or rule area means to the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZoneKind {
    /// A net's copper pour: fixed copper of that net, a Specctra `plane`.
    Plane,
    /// No tracks or vias.
    KeepoutAll,
    /// No tracks.
    KeepoutTracks,
    /// No vias.
    KeepoutVias,
}

#[derive(Debug, Clone)]
pub(crate) struct ZoneModel {
    kind: ZoneKind,
    layer: String,
    net: Option<String>,
    outline: Vec<(i64, i64)>,
}

#[derive(Debug, Clone, Default)]
struct LockedRouting {
    tracks: Vec<LockedTrackModel>,
    vias: Vec<LockedViaModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RuleKey {
    track_width_um: i64,
    clearance_um: i64,
    via_diameter_um: i64,
    via_drill_um: i64,
}

type RuleGroups = BTreeMap<RuleKey, Vec<String>>;
type NetClassNames = BTreeMap<String, String>;

#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    routing_context: Option<crate::routing_context::RoutingContext>,
    schema_version: u32,
    board_path: String,
    source_sha256: String,
    coordinate_unit: String,
    resolution: u32,
    supported_profile: SupportedProfile,
    layers: Vec<ManifestLayer>,
    components: Vec<ManifestComponent>,
    nets: Vec<ManifestNet>,
    padstacks: Vec<ManifestPadstack>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SupportedProfile {
    copper_layers: u32,
    component_side: String,
    pad_shapes: Vec<String>,
    existing_routing: bool,
    #[serde(default)]
    locked_track_count: usize,
    #[serde(default)]
    locked_via_count: usize,
    copper_zones: bool,
    /// Copper pours written as fixed planes, one per pour and layer.
    #[serde(default)]
    zone_plane_count: usize,
    /// Rule areas that forbid tracks or vias, one per area and layer.
    #[serde(default)]
    zone_keepout_count: usize,
    custom_rules: bool,
    outline: String,
    /// Footprints with no pads or holes, left out of the DSN.
    #[serde(default)]
    footprints_without_pads: usize,
    /// Keepouts standing for copper graphics and text, one per item and
    /// layer, on the board and in footprints.
    #[serde(default)]
    copper_graphic_keepout_count: usize,
    /// The board's pours were left out of the DSN (`zone_plane_count` still
    /// counts them); they need a refill after the import.
    #[serde(default)]
    pours_omitted: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct ManifestLayer {
    kicad_name: String,
    dsn_name: String,
    index: usize,
    /// `signal`, or `power` for a KiCad power layer the router keeps tracks off.
    #[serde(default = "signal_layer")]
    dsn_type: String,
}

fn signal_layer() -> String {
    "signal".to_string()
}

#[derive(Debug, Deserialize, Serialize)]
struct ManifestComponent {
    reference: String,
    kiid: String,
    image_name: String,
    x_um: i64,
    y_um: i64,
    rotation_degrees: f64,
    /// The side in the DSN, always `front` (see [`FootprintModel::kicad_side`]).
    side: String,
    /// The side in KiCad, `front` or `back`.
    #[serde(default = "front_side")]
    kicad_side: String,
    pads: Vec<ManifestPin>,
}

fn front_side() -> String {
    "front".to_string()
}

#[derive(Debug, Deserialize, Serialize)]
struct ManifestPin {
    pad_number: String,
    dsn_pin: String,
    net: Option<String>,
    padstack_name: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ManifestNet {
    name: String,
    pins: Vec<String>,
    class_name: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ManifestPadstack {
    name: String,
    purpose: String,
    shape: PadShape,
    layers: Vec<String>,
    size_x_um: i64,
    size_y_um: i64,
    drill_um: Option<i64>,
}

/// Choices the caller makes about what the router is given.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExportOptions {
    /// Leave copper pours out of the DSN, so the router may route through
    /// their area (refill them after the import). Rule areas stay.
    pub omit_pours: bool,
}

#[cfg(test)]
pub(crate) fn export_dsn(
    board_path: &Path,
    board_source: &str,
    effective_rules: &IpcEffectiveRoutingRules,
) -> Result<ExportBundle> {
    export_dsn_with_options(
        board_path,
        board_source,
        effective_rules,
        ExportOptions::default(),
    )
}

pub(crate) fn export_dsn_with_options(
    board_path: &Path,
    board_source: &str,
    effective_rules: &IpcEffectiveRoutingRules,
    options: ExportOptions,
) -> Result<ExportBundle> {
    let tree = parse_sexp(board_source).context("parse KiCad IPC board snapshot")?;
    if tree.head() != Some("kicad_pcb") {
        bail!("KiCad IPC snapshot root is not 'kicad_pcb'");
    }

    let copper_stack = copper_stack(&tree)?;
    let copper_layers: Vec<String> = copper_stack.iter().map(|(name, _)| name.clone()).collect();
    let outline = simple_closed_outline(&tree)?;
    let net_table = top_level_net_table(&tree);
    let locked_routing = locked_routing(&tree, &net_table, &copper_layers)?;
    let board_keepouts = board_copper_keepouts(&tree, &copper_layers)?;
    let (footprints, skipped_without_pads) = footprints(&tree, &net_table, &copper_layers)?;
    let zones = zones(&tree, &net_table, &copper_layers)?;
    if footprints.is_empty() {
        bail!("supported routing profile requires at least one footprint");
    }

    let mut net_pins: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for footprint in &footprints {
        for pad in &footprint.pads {
            if let Some(net) = &pad.net {
                net_pins
                    .entry(net.clone())
                    .or_default()
                    .push(format!("{}-{}", footprint.reference, pad.dsn_id));
            }
        }
    }
    if net_pins.is_empty() {
        bail!("supported routing profile requires at least one connected pad");
    }
    for pins in net_pins.values_mut() {
        pins.sort();
        pins.dedup();
    }

    // What the router is given: every rule area, and the pours unless the
    // caller left them out. The manifest still counts the board's pours, so
    // the importer can check the board kept them.
    let routed_zones: Vec<ZoneModel> = zones
        .iter()
        .filter(|zone| !(options.omit_pours && zone.kind == ZoneKind::Plane))
        .cloned()
        .collect();
    for zone in &routed_zones {
        if let Some(net) = &zone.net {
            if !net_pins.contains_key(net) {
                bail!(
                    "unsupported first routing profile: the copper zone on net '{net}' has no pad on that net, so the router cannot know what it connects"
                );
            }
        }
    }

    let (class_nets, net_classes) = normalize_rules(&net_pins, effective_rules)?;

    let mut padstack_keys = BTreeSet::new();
    for footprint in &footprints {
        for pad in &footprint.pads {
            padstack_keys.insert(pad.padstack.clone());
        }
    }
    let padstack_names: BTreeMap<PadstackKey, String> = padstack_keys
        .into_iter()
        .enumerate()
        .map(|(index, key)| (key, format!("konnect_pad_{:04}", index + 1)))
        .collect();

    let via_keys: BTreeSet<RuleKey> = class_nets.keys().cloned().collect();
    let via_names: BTreeMap<RuleKey, String> = via_keys
        .into_iter()
        .enumerate()
        .map(|(index, key)| (key, format!("konnect_via_{:04}", index + 1)))
        .collect();

    let pcb = build_pcb(
        board_path,
        &copper_stack,
        &outline,
        &footprints,
        &net_pins,
        &class_nets,
        &net_classes,
        &padstack_names,
        &via_names,
        &locked_routing,
        &routed_zones,
        &board_keepouts,
    )?;
    let dsn = serialize_and_validate(pcb)?;
    let source_sha256 = sha256_hex(board_source.as_bytes());
    let manifest = build_manifest(
        board_path,
        &source_sha256,
        options.omit_pours,
        skipped_without_pads,
        board_keepouts.len(),
        &copper_stack,
        &footprints,
        &net_pins,
        &net_classes,
        &padstack_names,
        &via_names,
        &locked_routing,
        &zones,
    )?;

    Ok(ExportBundle {
        dsn,
        manifest,
        source_sha256,
        component_count: footprints.len(),
        pad_count: footprints
            .iter()
            .map(|footprint| footprint.pads.len())
            .sum(),
        net_count: net_pins.len(),
        class_count: class_nets.len(),
    })
}

/// Production export binds saved sidecars and the original live class values.
pub(crate) fn export_dsn_with_context(
    board: &Path,
    source: &str,
    rules: &IpcEffectiveRoutingRules,
    snapshot: &crate::routing_context::Snapshot,
    options: ExportOptions,
) -> Result<ExportBundle> {
    let adjusted = snapshot.project_rules(rules)?;
    let mut bundle = export_dsn_with_options(board, source, &adjusted, options)?;
    let mut manifest: Manifest = serde_json::from_str(&bundle.manifest)?;
    manifest.routing_context = Some(snapshot.seal(rules)?);
    manifest.supported_profile.custom_rules = snapshot.rules.is_some();
    bundle.manifest = serde_json::to_string_pretty(&manifest)?;
    Ok(bundle)
}

/// Replace Konnect's deterministic DSN with a KiCad-native DSN while keeping
/// the same fail-closed board profile and rewriting the reverse manifest to
/// the exact identifiers KiCad emitted. This lets the strict SES importer
/// remain authoritative instead of trusting a second, looser return path.
pub(crate) fn adopt_native_dsn(
    mut baseline: ExportBundle,
    native_dsn: String,
) -> Result<ExportBundle> {
    {
        let manifest: Manifest =
            serde_json::from_str(&baseline.manifest).context("parse baseline routing manifest")?;
        if manifest.supported_profile.pours_omitted {
            bail!(
                "native DSN not adopted: the pours were left out, and KiCad's exporter always writes them"
            );
        }
        if let Some(back) = manifest.components.iter().find(|c| c.kicad_side == "back") {
            bail!(
                "native DSN not adopted: '{}' is a back-side part, which KiCad's exporter writes as \
                 a mirrored image and Konnect writes unmirrored at its true pad positions; use the \
                 Rust exporter (native_bridge_mode 'disable' or 'prefer')",
                back.reference
            );
        }
    }
    validate_dsn_syntax(&native_dsn, "KiCad native DSN")?;
    let baseline_tree = parse_dsn_sexp(&baseline.dsn).context("parse Konnect baseline DSN")?;
    let native_tree = parse_dsn_sexp(&native_dsn).context("parse KiCad native DSN")?;
    let baseline_identity = dsn_identity(&baseline_tree)?;
    let native_identity = dsn_identity(&native_tree)?;
    let mut manifest: Manifest =
        serde_json::from_str(&baseline.manifest).context("parse baseline routing manifest")?;
    let layer_names = correlate_layers(&baseline_identity, &native_identity)?;
    let (class_names, via_names) = correlate_classes(&baseline_identity, &native_identity)?;
    let padstack_names = correlate_components(&baseline_identity, &native_identity)?;
    validate_native_nets(&baseline_identity, &native_identity)?;

    for layer in &mut manifest.layers {
        layer.dsn_name = layer_names
            .get(&layer.dsn_name)
            .with_context(|| format!("native DSN omitted layer '{}'", layer.dsn_name))?
            .clone();
    }
    for component in &mut manifest.components {
        let native = native_identity
            .components
            .get(&component.reference)
            .with_context(|| format!("native DSN omitted component '{}'", component.reference))?;
        component.image_name = native.image_name.clone();
        for pin in &mut component.pads {
            pin.padstack_name = padstack_names
                .get(&pin.padstack_name)
                .with_context(|| {
                    format!(
                        "native DSN omitted padstack mapping for {}-{}",
                        component.reference, pin.pad_number
                    )
                })?
                .clone();
        }
    }
    for net in &mut manifest.nets {
        net.class_name = class_names
            .get(&net.class_name)
            .with_context(|| format!("native DSN omitted class mapping for '{}'", net.name))?
            .clone();
    }
    for padstack in &mut manifest.padstacks {
        let mapping = if padstack.purpose == "via" {
            &via_names
        } else {
            &padstack_names
        };
        padstack.name = mapping
            .get(&padstack.name)
            .with_context(|| format!("native DSN omitted padstack '{}'", padstack.name))?
            .clone();
    }
    manifest
        .padstacks
        .sort_by(|left, right| left.name.cmp(&right.name));
    baseline.manifest =
        serde_json::to_string_pretty(&manifest).context("serialize native routing manifest")?;
    baseline.dsn = native_dsn;
    Ok(baseline)
}

#[derive(Debug, Clone)]
struct DsnPlacementIdentity {
    image_name: String,
    x: i64,
    y: i64,
    side: String,
    rotation: OrderedF64,
    pins: BTreeMap<String, DsnPinIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DsnPinIdentity {
    padstack: String,
    x: i64,
    y: i64,
    /// The pin's `(rotate r)`, if it has one.
    rotation: Option<OrderedF64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct OrderedF64(u64);

impl OrderedF64 {
    fn new(value: f64) -> Result<Self> {
        if !value.is_finite() {
            bail!("DSN contains a non-finite number");
        }
        Ok(Self(value.to_bits()))
    }
}

#[derive(Debug, Clone)]
struct DsnClassIdentity {
    nets: BTreeSet<String>,
    via_name: String,
    width: i64,
    clearance: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DsnShapeIdentity {
    Circle {
        layer: String,
        diameter: i64,
    },
    Rect {
        layer: String,
        x1: i64,
        y1: i64,
        x2: i64,
        y2: i64,
    },
}

#[derive(Debug, Clone)]
struct DsnIdentity {
    layers: BTreeMap<usize, String>,
    components: BTreeMap<String, DsnPlacementIdentity>,
    nets: BTreeMap<String, BTreeSet<String>>,
    classes: BTreeMap<String, DsnClassIdentity>,
    padstacks: BTreeMap<String, Vec<DsnShapeIdentity>>,
}

fn validate_dsn_syntax(source: &str, label: &str) -> Result<()> {
    let cursor = Cursor::new(source.as_bytes());
    let mut tokenizer = ListTokenizer::new(BufReader::new(cursor));
    dsn::DsnFile::read_dsn(&mut tokenizer)
        .map_err(|error| anyhow::anyhow!("{label} failed Specctra parser validation: {error}"))?;
    Ok(())
}

fn parse_dsn_sexp(source: &str) -> Result<SexpNode> {
    // Specctra represents the quote delimiter as the bare token
    // `(string_quote ")`. KiCad's S-expression parser correctly treats the
    // opening quote as a string delimiter, so give only this metadata token an
    // escaped representation for structural inspection. The original DSN is
    // retained byte-for-byte and independently validated by Topola's parser.
    let compatible = source.replace("(string_quote \")", "(string_quote \"\\\"\")");
    parse_sexp(&compatible).context("parse normalized Specctra S-expression")
}

fn dsn_identity(root: &SexpNode) -> Result<DsnIdentity> {
    if root.head() != Some("pcb") {
        bail!("Specctra export root is not 'pcb'");
    }
    let structure = root.find("structure").context("DSN has no structure")?;
    let mut layers = BTreeMap::new();
    for layer in structure.find_all("layer") {
        let name = dsn_atom(layer, 1, "layer name")?.to_string();
        let index = layer
            .find("property")
            .and_then(|property| property.find("index"))
            .and_then(|index| index.get(1))
            .and_then(SexpNode::as_str)
            .context("DSN layer has no index")?
            .parse::<usize>()
            .context("DSN layer index is not an integer")?;
        if layers.insert(index, name).is_some() {
            bail!("DSN repeats a layer index");
        }
    }

    let library = root.find("library").context("DSN has no library")?;
    let mut image_pins = BTreeMap::<String, BTreeMap<String, DsnPinIdentity>>::new();
    for image in library.find_all("image") {
        let name = dsn_atom(image, 1, "image name")?.to_string();
        let mut pins = BTreeMap::new();
        for pin in image.find_all("pin") {
            // `(pin padstack [(rotate r)] number x y)`: the optional rotation
            // is a sub-list between the padstack and the number.
            let rotation = match pin.find("rotate") {
                Some(rotate) => Some(OrderedF64::new(dsn_number(rotate, 1, "pin rotation")?)?),
                None => None,
            };
            let positional = SexpNode::List(
                pin.children()
                    .unwrap_or(&[])
                    .iter()
                    .filter(|child| child.head() != Some("rotate"))
                    .cloned()
                    .collect(),
            );
            let pin = &positional;
            let padstack = dsn_atom(pin, 1, "pin padstack")?.to_string();
            let number = dsn_atom(pin, 2, "pin number")?.to_string();
            let identity = DsnPinIdentity {
                padstack,
                x: dsn_integer(pin, 3, "pin x")?,
                y: dsn_integer(pin, 4, "pin y")?,
                rotation,
            };
            if pins.insert(number, identity).is_some() {
                bail!("DSN image '{name}' repeats a pin number");
            }
        }
        if image_pins.insert(name.clone(), pins).is_some() {
            bail!("DSN repeats image '{name}'");
        }
    }
    let mut padstacks = BTreeMap::new();
    for padstack in library.find_all("padstack") {
        let name = dsn_atom(padstack, 1, "padstack name")?.to_string();
        let shapes = padstack
            .find_all("shape")
            .into_iter()
            .map(|shape| {
                let geometry = shape.get(1).context("DSN shape has no geometry")?;
                match geometry.head() {
                    Some("circle") => Ok(DsnShapeIdentity::Circle {
                        layer: dsn_atom(geometry, 1, "circle layer")?.to_string(),
                        diameter: dsn_integer(geometry, 2, "circle diameter")?,
                    }),
                    Some("rect") => Ok(DsnShapeIdentity::Rect {
                        layer: dsn_atom(geometry, 1, "rect layer")?.to_string(),
                        x1: dsn_integer(geometry, 2, "rect x1")?,
                        y1: dsn_integer(geometry, 3, "rect y1")?,
                        x2: dsn_integer(geometry, 4, "rect x2")?,
                        y2: dsn_integer(geometry, 5, "rect y2")?,
                    }),
                    other => bail!("DSN padstack has unsupported shape {other:?}"),
                }
            })
            .collect::<Result<Vec<_>>>()?;
        if shapes.is_empty() || padstacks.insert(name.clone(), shapes).is_some() {
            bail!("DSN contains an empty or duplicate padstack '{name}'");
        }
    }

    let placement = root.find("placement").context("DSN has no placement")?;
    let mut components = BTreeMap::new();
    for component in placement.find_all("component") {
        let image_name = dsn_atom(component, 1, "component image")?.to_string();
        let pins = image_pins
            .get(&image_name)
            .with_context(|| format!("DSN placement uses unknown image '{image_name}'"))?
            .clone();
        for place in component.find_all("place") {
            let reference = dsn_atom(place, 1, "place reference")?.to_string();
            let x = dsn_integer(place, 2, "place x")?;
            let y = dsn_integer(place, 3, "place y")?;
            let side = dsn_atom(place, 4, "place side")?.to_string();
            let rotation = OrderedF64::new(dsn_number(place, 5, "place rotation")?)?;
            let identity = DsnPlacementIdentity {
                image_name: image_name.clone(),
                x,
                y,
                side,
                rotation,
                pins: pins.clone(),
            };
            if components.insert(reference.clone(), identity).is_some() {
                bail!("DSN repeats component '{reference}'");
            }
        }
    }

    let network = root.find("network").context("DSN has no network")?;
    let mut nets = BTreeMap::new();
    for net in network.find_all("net") {
        let name = dsn_atom(net, 1, "net name")?.to_string();
        let pins = net
            .find("pins")
            .context("DSN net has no pins")?
            .children()
            .context("DSN pins is not a list")?
            .iter()
            .skip(1)
            .map(|pin| {
                pin.as_str()
                    .map(str::to_string)
                    .context("DSN pins contains a list")
            })
            .collect::<Result<BTreeSet<_>>>()?;
        if pins.is_empty() || nets.insert(name.clone(), pins).is_some() {
            bail!("DSN contains an empty or duplicate net '{name}'");
        }
    }
    let mut classes = BTreeMap::new();
    for class in network.find_all("class") {
        let name = dsn_atom(class, 1, "class name")?.to_string();
        let children = class.children().context("DSN class is not a list")?;
        let nets = children
            .iter()
            .skip(2)
            .take_while(|child| child.as_str().is_some())
            .map(|child| child.as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>();
        let via_name = class
            .find("circuit")
            .and_then(|circuit| circuit.find("use_via"))
            .and_then(|via| via.get(1))
            .and_then(SexpNode::as_str)
            .context("DSN class has no use_via")?
            .to_string();
        let rule = class.find("rule").context("DSN class has no rule")?;
        let width = rule.find("width").context("DSN class has no width")?;
        let clearance = rule
            .find("clearance")
            .context("DSN class has no clearance")?;
        let width = dsn_integer(width, 1, "class width")?;
        let clearance = dsn_integer(clearance, 1, "class clearance")?;
        if nets.is_empty()
            || !nets.iter().all(|net| {
                network
                    .find_all("net")
                    .iter()
                    .any(|node| node.get(1).and_then(SexpNode::as_str) == Some(net))
            })
            || classes
                .insert(
                    name.clone(),
                    DsnClassIdentity {
                        nets,
                        via_name,
                        width,
                        clearance,
                    },
                )
                .is_some()
        {
            bail!("DSN contains an invalid or duplicate class '{name}'");
        }
    }
    Ok(DsnIdentity {
        layers,
        components,
        nets,
        classes,
        padstacks,
    })
}

fn correlate_layers(
    baseline: &DsnIdentity,
    native: &DsnIdentity,
) -> Result<BTreeMap<String, String>> {
    if baseline.layers.len() != native.layers.len() {
        bail!("native DSN changed the copper-layer count");
    }
    let mappings = baseline
        .layers
        .iter()
        .map(|(index, baseline_name)| {
            let native_name = native
                .layers
                .get(index)
                .with_context(|| format!("native DSN omitted layer index {index}"))?;
            Ok((baseline_name.clone(), native_name.clone()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    if mappings.iter().any(|(baseline, native)| baseline != native) {
        bail!("native DSN renamed a KiCad copper layer");
    }
    Ok(mappings)
}

fn correlate_components(
    baseline: &DsnIdentity,
    native: &DsnIdentity,
) -> Result<BTreeMap<String, String>> {
    if baseline.components.len() != native.components.len() {
        bail!("native DSN changed the component count");
    }
    let mut padstacks = BTreeMap::new();
    for (reference, baseline_component) in &baseline.components {
        let native_component = native
            .components
            .get(reference)
            .with_context(|| format!("native DSN omitted component '{reference}'"))?;
        if baseline_component.x != native_component.x
            || baseline_component.y != native_component.y
            || baseline_component.side != native_component.side
            || baseline_component.rotation != native_component.rotation
            || baseline_component.pins.keys().collect::<Vec<_>>()
                != native_component.pins.keys().collect::<Vec<_>>()
        {
            bail!("native DSN changed placement or pins for component '{reference}'");
        }
        for (pin, baseline_pin) in &baseline_component.pins {
            let native_pin = native_component.pins.get(pin).unwrap();
            if baseline_pin.x != native_pin.x || baseline_pin.y != native_pin.y {
                bail!("native DSN changed pin position for component '{reference}' pin '{pin}'");
            }
            let degrees =
                |r: Option<OrderedF64>| r.map_or(0.0, |r| f64::from_bits(r.0)).rem_euclid(360.0);
            if (degrees(baseline_pin.rotation) - degrees(native_pin.rotation)).abs() > 1e-9 {
                bail!("native DSN changed pin rotation for component '{reference}' pin '{pin}'");
            }
            let baseline_shape = baseline
                .padstacks
                .get(&baseline_pin.padstack)
                .with_context(|| {
                    format!("baseline DSN omitted padstack '{}'", baseline_pin.padstack)
                })?;
            let native_shape = native
                .padstacks
                .get(&native_pin.padstack)
                .with_context(|| {
                    format!("native DSN omitted padstack '{}'", native_pin.padstack)
                })?;
            if baseline_shape != native_shape {
                bail!("native DSN changed pad geometry for component '{reference}' pin '{pin}'");
            }
            if padstacks
                .insert(baseline_pin.padstack.clone(), native_pin.padstack.clone())
                .is_some_and(|previous| previous != native_pin.padstack)
            {
                bail!("native DSN maps one pad geometry to inconsistent padstacks");
            }
        }
    }
    Ok(padstacks)
}

fn validate_native_nets(baseline: &DsnIdentity, native: &DsnIdentity) -> Result<()> {
    if baseline.nets != native.nets {
        bail!("native DSN changed net or pin membership");
    }
    Ok(())
}

fn correlate_classes(
    baseline: &DsnIdentity,
    native: &DsnIdentity,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    if baseline.classes.len() != native.classes.len() {
        bail!("native DSN changed the routing-class count");
    }
    let mut class_names = BTreeMap::new();
    let mut via_names = BTreeMap::new();
    for (baseline_name, baseline_class) in &baseline.classes {
        let matches = native
            .classes
            .iter()
            .filter(|(_, native_class)| native_class.nets == baseline_class.nets)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            bail!("native DSN has no unique class for '{baseline_name}'");
        }
        let (native_name, native_class) = matches[0];
        if baseline_class.width != native_class.width
            || baseline_class.clearance != native_class.clearance
        {
            bail!("native DSN changed width or clearance for class '{baseline_name}'");
        }
        let baseline_via = baseline
            .padstacks
            .get(&baseline_class.via_name)
            .with_context(|| format!("baseline DSN omitted via '{}'", baseline_class.via_name))?;
        let native_via = native
            .padstacks
            .get(&native_class.via_name)
            .with_context(|| format!("native DSN omitted via '{}'", native_class.via_name))?;
        if baseline_via != native_via {
            bail!("native DSN changed via geometry for class '{baseline_name}'");
        }
        class_names.insert(baseline_name.clone(), native_name.clone());
        if via_names
            .insert(
                baseline_class.via_name.clone(),
                native_class.via_name.clone(),
            )
            .is_some_and(|previous| previous != native_class.via_name)
        {
            bail!("native DSN maps one via rule to inconsistent padstacks");
        }
    }
    Ok((class_names, via_names))
}

fn dsn_atom<'a>(node: &'a SexpNode, index: usize, label: &str) -> Result<&'a str> {
    node.get(index)
        .and_then(SexpNode::as_str)
        .with_context(|| format!("DSN is missing {label}"))
}

fn dsn_number(node: &SexpNode, index: usize, label: &str) -> Result<f64> {
    let value = dsn_atom(node, index, label)?
        .parse::<f64>()
        .with_context(|| format!("DSN {label} is not numeric"))?;
    if !value.is_finite() {
        bail!("DSN {label} is not finite");
    }
    Ok(value)
}

fn dsn_integer(node: &SexpNode, index: usize, label: &str) -> Result<i64> {
    let value = dsn_number(node, index, label)?;
    if value.fract() != 0.0 || value < i64::MIN as f64 || value > i64::MAX as f64 {
        bail!("DSN {label} is not a supported integer");
    }
    Ok(value as i64)
}

fn locked_routing(
    tree: &SexpNode,
    net_table: &BTreeMap<String, String>,
    copper_layers: &[String],
) -> Result<LockedRouting> {
    let arc_count = tree.find_all("arc").len();
    if arc_count > 0 {
        let locked = tree
            .find_all("arc")
            .iter()
            .filter(|arc| arc.find_str("locked") == Some("yes"))
            .count();
        if locked > 0 {
            bail!(
                "unsupported first routing profile: board contains {locked} locked routed arc(s), which cannot be represented without approximation"
            );
        }
        bail!(
            "unsupported first routing profile: board contains {arc_count} existing routed arc(s)"
        );
    }

    let mut routing = LockedRouting::default();
    for segment in tree.find_all("segment") {
        if segment.find_str("locked") != Some("yes") {
            bail!("unsupported first routing profile: board contains an existing track segment that is not locked");
        }
        let layer = segment
            .find_str("layer")
            .context("locked track segment has no layer")?
            .to_string();
        if !copper_layers.contains(&layer) {
            bail!("locked track segment uses unsupported layer '{layer}'");
        }
        let net = segment
            .find("net")
            .and_then(|node| resolve_net(node, net_table))
            .context("locked track segment has no connected net")?;
        let (start_x_um, start_y_um) = point_um(segment, "start")?;
        let (end_x_um, end_y_um) = point_um(segment, "end")?;
        if start_x_um == end_x_um && start_y_um == end_y_um {
            bail!("locked track segment has zero length");
        }
        routing.tracks.push(LockedTrackModel {
            start_x_um,
            start_y_um,
            end_x_um,
            end_y_um,
            width_um: positive_um(
                segment.find("width").and_then(|width| width.get_f64(1)),
                "locked track width",
            )?,
            layer,
            net,
        });
    }
    for via in tree.find_all("via") {
        if via.find_str("locked") != Some("yes") {
            bail!("unsupported first routing profile: board contains an existing via that is not locked");
        }
        if let Some(kind) = via.find_str("type") {
            bail!("unsupported locked via type '{kind}'; only through vias are supported");
        }
        let layers = via
            .find("layers")
            .and_then(SexpNode::children)
            .unwrap_or(&[])
            .iter()
            .skip(1)
            .filter_map(SexpNode::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        // KiCad writes a through via as its outer layers whatever the stack.
        let through = [
            copper_layers.first().cloned().unwrap_or_default(),
            copper_layers.last().cloned().unwrap_or_default(),
        ];
        if layers != through {
            bail!(
                "unsupported locked via layer span [{}]; expected the through span [{}]",
                layers.join(", "),
                through.join(", ")
            );
        }
        let net = via
            .find("net")
            .and_then(|node| resolve_net(node, net_table))
            .context("locked via has no connected net")?;
        let (x_um, y_um) = point_um(via, "at")?;
        let diameter_um = positive_um(
            via.find("size").and_then(|size| size.get_f64(1)),
            "locked via diameter",
        )?;
        let drill = via.find("drill").context("locked via has no drill")?;
        if drill.get(1).and_then(SexpNode::as_str) == Some("oval") {
            bail!("unsupported locked via with oval drill");
        }
        let drill_um = positive_um(drill.get_f64(1), "locked via drill")?;
        if drill_um >= diameter_um {
            bail!("locked via drill is not smaller than its diameter");
        }
        routing.vias.push(LockedViaModel {
            x_um,
            y_um,
            diameter_um,
            drill_um,
            net,
        });
    }
    routing.tracks.sort_by(|left, right| {
        (
            &left.net,
            &left.layer,
            left.start_x_um,
            left.start_y_um,
            left.end_x_um,
            left.end_y_um,
        )
            .cmp(&(
                &right.net,
                &right.layer,
                right.start_x_um,
                right.start_y_um,
                right.end_x_um,
                right.end_y_um,
            ))
    });
    routing.vias.sort_by(|left, right| {
        (&left.net, left.x_um, left.y_um).cmp(&(&right.net, right.x_um, right.y_um))
    });
    Ok(routing)
}

/// The most copper layers KiCad allows.
const MAX_COPPER_LAYERS: usize = 32;

/// The board's copper layers in stack order (`F.Cu`, `In1.Cu` … `InN.Cu`,
/// `B.Cu`), each with its Specctra layer type: KiCad `power` layers are
/// `power` (the router keeps tracks off them), `signal` and `mixed` are
/// `signal`. A `jumper` layer, a gap in the inner numbering, an odd count or
/// a missing outer layer is refused.
fn copper_stack(tree: &SexpNode) -> Result<Vec<(String, &'static str)>> {
    let layers = tree.find("layers").context("board has no layers table")?;
    let mut rows: Vec<(String, String)> = Vec::new();
    for row in layers.children().unwrap_or(&[]).iter().skip(1) {
        // Layer table rows are `(numeric-id "name" type ["user name"])`, so the
        // canonical name is the first data item even though the numeric id is
        // the list head. A user name never replaces it.
        let Some(name) = row.get(1).and_then(SexpNode::as_str) else {
            continue;
        };
        if !name.ends_with(".Cu") {
            continue;
        }
        let kind = row.get(2).and_then(SexpNode::as_str).unwrap_or("signal");
        if rows.iter().any(|(seen, _)| seen == name) {
            bail!("board layer table repeats copper layer '{name}'");
        }
        rows.push((name.to_string(), kind.to_string()));
    }
    let inner_index = |name: &str| -> Option<usize> {
        name.strip_prefix("In")?.strip_suffix(".Cu")?.parse().ok()
    };
    let has = |name: &str| rows.iter().any(|(n, _)| n == name);
    if !has("F.Cu") || !has("B.Cu") {
        bail!(
            "unsupported routing profile: copper layers [{}] do not include both F.Cu and B.Cu",
            rows.iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut inner: Vec<usize> = Vec::new();
    for (name, _) in &rows {
        if name == "F.Cu" || name == "B.Cu" {
            continue;
        }
        let index = inner_index(name)
            .with_context(|| format!("unsupported routing profile: copper layer '{name}'"))?;
        inner.push(index);
    }
    inner.sort_unstable();
    if inner.iter().enumerate().any(|(i, index)| *index != i + 1) {
        bail!(
            "unsupported routing profile: inner copper layers are not In1.Cu to In{}.Cu without gaps",
            inner.len()
        );
    }
    let count = inner.len() + 2;
    if !count.is_multiple_of(2) || count > MAX_COPPER_LAYERS {
        bail!("unsupported routing profile: {count} copper layers (KiCad uses an even count, 2 to 32)");
    }
    let mut order = vec!["F.Cu".to_string()];
    order.extend((1..=inner.len()).map(|i| format!("In{i}.Cu")));
    order.push("B.Cu".to_string());
    order
        .into_iter()
        .map(|name| {
            let kind = &rows
                .iter()
                .find(|(n, _)| *n == name)
                .expect("listed above")
                .1;
            let dsn = match kind.as_str() {
                "signal" | "mixed" => "signal",
                "power" => "power",
                other => {
                    bail!("unsupported routing profile: copper layer '{name}' has type '{other}'")
                }
            };
            Ok((name, dsn))
        })
        .collect()
}

fn copper_layers(tree: &SexpNode) -> Result<Vec<String>> {
    Ok(copper_stack(tree)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

/// One open Edge.Cuts piece: a line or an arc, with the chord points between
/// its ends for an arc.
struct OutlineEdge {
    start: (i64, i64),
    end: (i64, i64),
    /// Points strictly between `start` and `end`, in that order.
    inner: Vec<(i64, i64)>,
}

/// A KiCad `(x y)` pair in millimetres, as KiCad writes it (Y grows downward).
fn xy_mm(node: &SexpNode, tag: &str) -> Result<(f64, f64)> {
    let point = node.find(tag).with_context(|| format!("missing '{tag}'"))?;
    Ok((
        finite_number(point.get_f64(1), tag)?,
        finite_number(point.get_f64(2), tag)?,
    ))
}

/// Millimetres in KiCad's orientation to the DSN's micrometres, Y up.
fn to_dsn(point: (f64, f64)) -> Result<(i64, i64)> {
    Ok((
        finite_um(Some(point.0), "outline x")?,
        -finite_um(Some(point.1), "outline y")?,
    ))
}

/// The points strictly between `start` and `end` of the arc through `mid`,
/// on the arc, close enough that no chord strays more than
/// [`ARC_TOLERANCE_MM`] from it. Three collinear points are a straight line.
fn arc_inner_points(
    start: (f64, f64),
    mid: (f64, f64),
    end: (f64, f64),
) -> Result<Vec<(f64, f64)>> {
    let (ax, ay) = start;
    let (bx, by) = mid;
    let (cx, cy) = end;
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-9 {
        return Ok(Vec::new());
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    let radius = ((ax - ux).powi(2) + (ay - uy).powi(2)).sqrt();
    if !radius.is_finite() || radius <= 0.0 {
        bail!("unsupported outline: an Edge.Cuts arc has no usable radius");
    }
    let angle = |(x, y): (f64, f64)| (y - uy).atan2(x - ux);
    let tau = std::f64::consts::TAU;
    // The side the middle point is on says which way round the arc goes.
    let counter_clockwise = (bx - ax) * (cy - by) - (by - ay) * (cx - bx) > 0.0;
    let from = angle(start);
    let to = angle(end);
    let sweep = if counter_clockwise {
        (to - from).rem_euclid(tau)
    } else {
        -((from - to).rem_euclid(tau))
    };
    chord_points(from, sweep, (ux, uy), radius)
}

/// `count - 1` points at equal angles along a sweep from `from`, on the circle.
fn chord_points(from: f64, sweep: f64, centre: (f64, f64), radius: f64) -> Result<Vec<(f64, f64)>> {
    let step = if ARC_TOLERANCE_MM >= radius {
        std::f64::consts::PI
    } else {
        2.0 * (1.0 - ARC_TOLERANCE_MM / radius).acos()
    };
    let segments = (sweep.abs() / step).ceil().max(1.0);
    if segments > MAX_ARC_SEGMENTS as f64 {
        bail!(
            "unsupported outline: an arc or circle would need more than {MAX_ARC_SEGMENTS} chords"
        );
    }
    let segments = segments as usize;
    Ok((1..segments)
        .map(|i| {
            let theta = from + sweep * i as f64 / segments as f64;
            (
                centre.0 + radius * theta.cos(),
                centre.1 + radius * theta.sin(),
            )
        })
        .collect())
}

/// The points of a full circle, `radius` mm round `centre`, as a closed loop's
/// distinct vertices.
fn circle_points(centre: (f64, f64), radius: f64) -> Result<Vec<(f64, f64)>> {
    if !(radius.is_finite() && radius > 0.0) {
        bail!("unsupported outline: an Edge.Cuts circle has no radius");
    }
    let mut points = vec![(centre.0 + radius, centre.1)];
    points.extend(chord_points(0.0, std::f64::consts::TAU, centre, radius)?);
    Ok(points)
}

fn simple_closed_outline(tree: &SexpNode) -> Result<Vec<(i64, i64)>> {
    let mut edges: Vec<OutlineEdge> = Vec::new();
    // Shapes that close themselves: a rectangle, a circle, a polygon.
    let mut closed: Vec<Vec<(i64, i64)>> = Vec::new();
    for node in tree.children().unwrap_or(&[]) {
        if node.find_str("layer") != Some("Edge.Cuts") {
            continue;
        }
        match node.head() {
            Some("gr_line") => {
                let start = point_um(node, "start").context("Edge.Cuts line has no start")?;
                let end = point_um(node, "end").context("Edge.Cuts line has no end")?;
                if start == end {
                    bail!("unsupported outline: zero-length Edge.Cuts line");
                }
                edges.push(OutlineEdge {
                    start,
                    end,
                    inner: Vec::new(),
                });
            }
            Some("gr_arc") => {
                if node.find("mid").is_none() {
                    bail!(
                        "unsupported outline: an Edge.Cuts arc is in the old centre-and-angle form; open and save the board in KiCad 10 first"
                    );
                }
                let (start, mid, end) = (
                    xy_mm(node, "start")?,
                    xy_mm(node, "mid")?,
                    xy_mm(node, "end")?,
                );
                let (start_um, end_um) = (to_dsn(start)?, to_dsn(end)?);
                if start_um == end_um {
                    bail!("unsupported outline: an Edge.Cuts arc starts and ends at one point");
                }
                let inner = arc_inner_points(start, mid, end)?
                    .into_iter()
                    .map(to_dsn)
                    .collect::<Result<Vec<_>>>()?;
                edges.push(OutlineEdge {
                    start: start_um,
                    end: end_um,
                    inner,
                });
            }
            Some("gr_rect") => {
                let (a, b) = (xy_mm(node, "start")?, xy_mm(node, "end")?);
                if a.0 == b.0 || a.1 == b.1 {
                    bail!("unsupported outline: zero-area Edge.Cuts rectangle");
                }
                closed.push(
                    [a, (b.0, a.1), b, (a.0, b.1)]
                        .into_iter()
                        .map(to_dsn)
                        .collect::<Result<_>>()?,
                );
            }
            Some("gr_circle") => {
                let centre = xy_mm(node, "center")?;
                let edge = xy_mm(node, "end")?;
                let radius = ((edge.0 - centre.0).powi(2) + (edge.1 - centre.1).powi(2)).sqrt();
                closed.push(
                    circle_points(centre, radius)?
                        .into_iter()
                        .map(to_dsn)
                        .collect::<Result<_>>()?,
                );
            }
            Some("gr_poly") => {
                let points = node
                    .find("pts")
                    .context("Edge.Cuts polygon has no points")?
                    .find_all("xy")
                    .into_iter()
                    .map(|xy| {
                        to_dsn((
                            finite_number(xy.get_f64(1), "outline x")?,
                            finite_number(xy.get_f64(2), "outline y")?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                closed.push(points);
            }
            Some(tag) if tag.starts_with("gr_") => {
                bail!(
                    "unsupported first routing profile: Edge.Cuts '{tag}' is not a line, arc, rectangle, circle or polygon"
                )
            }
            _ => {}
        }
    }

    let ordered = if let Some(shape) = closed.pop() {
        if !closed.is_empty() || !edges.is_empty() {
            bail!("unsupported outline: Edge.Cuts contains more than one loop");
        }
        let mut shape = shape;
        shape.dedup();
        if shape.first() == shape.last() && shape.len() > 1 {
            shape.pop();
        }
        if shape.len() < 3 {
            bail!("unsupported outline: need at least three Edge.Cuts vertices");
        }
        let first = shape[0];
        shape.push(first);
        shape
    } else {
        chain_edges(&edges)?
    };
    // A loop that folds back on itself has no area for the router to use.
    let doubled_area: i128 = ordered
        .windows(2)
        .map(|pair| {
            i128::from(pair[0].0) * i128::from(pair[1].1)
                - i128::from(pair[1].0) * i128::from(pair[0].1)
        })
        .sum();
    if doubled_area == 0 {
        bail!("unsupported outline: Edge.Cuts encloses no area");
    }
    Ok(ordered)
}

/// Join open pieces end to end into one closed loop of points, the first point
/// repeated at the end.
fn chain_edges(edges: &[OutlineEdge]) -> Result<Vec<(i64, i64)>> {
    if edges.len() < 2 {
        bail!("unsupported outline: need at least three Edge.Cuts lines");
    }
    let mut at: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        at.entry(edge.start).or_default().push(index);
        at.entry(edge.end).or_default().push(index);
    }
    for (point, ends) in &at {
        if ends.len() != 2 {
            bail!(
                "unsupported outline: vertex ({}, {}) has degree {}, expected 2",
                point.0,
                point.1,
                ends.len()
            );
        }
    }

    let origin = edges[0].start;
    let mut ordered = vec![origin];
    let mut used = vec![false; edges.len()];
    let mut current = origin;
    let mut edge_index = 0;
    loop {
        used[edge_index] = true;
        let edge = &edges[edge_index];
        // Walk the piece from the end we stand on.
        let (to, inner): (_, Vec<_>) = if edge.start == current {
            (edge.end, edge.inner.clone())
        } else {
            (edge.start, edge.inner.iter().rev().copied().collect())
        };
        ordered.extend(inner);
        ordered.push(to);
        current = to;
        if current == origin {
            break;
        }
        match at[&current].iter().find(|index| !used[**index]) {
            Some(next) => edge_index = *next,
            None => bail!("unsupported outline: Edge.Cuts is not one closed loop"),
        }
    }
    if used.iter().any(|used| !used) {
        bail!("unsupported outline: Edge.Cuts contains more than one loop");
    }
    Ok(ordered)
}

/// The board's copper zones and rule areas as the router sees them.
///
/// A pour with a net is fixed copper of that net (a Specctra `plane`, which is
/// how KiCad's own DSN export writes one). A rule area that forbids tracks or
/// vias is a keepout. The zone's outline is used, not its fill, so thermal
/// reliefs and clearance gaps are not modelled: copper the fill will not reach
/// can still look connected to the router, which is why DRC after a refill is
/// the check. Anything else is refused rather than guessed at.
pub(crate) fn zones(
    tree: &SexpNode,
    net_table: &BTreeMap<String, String>,
    copper_layers: &[String],
) -> Result<Vec<ZoneModel>> {
    let mut out = Vec::new();
    for zone in tree.find_all("zone") {
        let name = zone.find_str("name").unwrap_or("unnamed");
        let mut layers: Vec<String> = Vec::new();
        if let Some(single) = zone.find_str("layer") {
            layers.push(single.to_string());
        }
        if let Some(list) = zone.find("layers") {
            layers.extend(
                list.children()
                    .unwrap_or(&[])
                    .iter()
                    .skip(1)
                    .filter_map(SexpNode::as_str)
                    .map(str::to_string),
            );
        }
        let mut copper: Vec<String> = Vec::new();
        for layer in &layers {
            if layer == "*.Cu" {
                copper.extend(copper_layers.iter().cloned());
            } else if copper_layers.contains(layer) {
                copper.push(layer.clone());
            }
        }
        copper.sort();
        copper.dedup();

        let kind = match zone.find("keepout") {
            Some(keepout) => {
                let blocked = |tag: &str| keepout.find_str(tag) == Some("not_allowed");
                match (blocked("tracks"), blocked("vias")) {
                    (true, true) => ZoneKind::KeepoutAll,
                    (true, false) => ZoneKind::KeepoutTracks,
                    (false, true) => ZoneKind::KeepoutVias,
                    // Forbids pours, pads or footprints only: nothing a router places.
                    (false, false) => continue,
                }
            }
            None => ZoneKind::Plane,
        };
        if copper.is_empty() {
            continue;
        }
        let net = match kind {
            ZoneKind::Plane => Some(
                zone.find("net")
                    .and_then(|node| resolve_net(node, net_table))
                    .with_context(|| {
                        format!(
                            "unsupported first routing profile: copper zone '{name}' has no net, so the router cannot tell what it connects"
                        )
                    })?,
            ),
            _ => None,
        };
        let outline = zone_outline(zone)
            .with_context(|| format!("unsupported first routing profile: zone '{name}'"))?;
        for layer in copper {
            out.push(ZoneModel {
                kind,
                layer,
                net: net.clone(),
                outline: outline.clone(),
            });
        }
    }
    Ok(out)
}

/// A zone's outline, closed, in DSN micrometres. Arcs in it are written as chords.
fn zone_outline(zone: &SexpNode) -> Result<Vec<(i64, i64)>> {
    let polygons = zone.find_all("polygon");
    let [polygon] = polygons.as_slice() else {
        bail!("expected one outline polygon, found {}", polygons.len());
    };
    let pts = polygon.find("pts").context("outline has no points")?;
    let mut points: Vec<(f64, f64)> = Vec::new();
    for item in pts.children().unwrap_or(&[]).iter().skip(1) {
        match item.head() {
            Some("xy") => points.push((
                finite_number(item.get_f64(1), "zone x")?,
                finite_number(item.get_f64(2), "zone y")?,
            )),
            Some("arc") => {
                let (start, mid, end) = (
                    xy_mm(item, "start")?,
                    xy_mm(item, "mid")?,
                    xy_mm(item, "end")?,
                );
                if points.last() != Some(&start) {
                    points.push(start);
                }
                points.extend(arc_inner_points(start, mid, end)?);
                points.push(end);
            }
            Some(other) => bail!("outline holds an unsupported '{other}'"),
            None => {}
        }
    }
    let mut outline = points.into_iter().map(to_dsn).collect::<Result<Vec<_>>>()?;
    outline.dedup();
    if outline.first() == outline.last() && outline.len() > 1 {
        outline.pop();
    }
    if outline.len() < 3 {
        bail!("outline has fewer than three corners");
    }
    let first = outline[0];
    outline.push(first);
    Ok(outline)
}

fn zone_polygon(zone: &ZoneModel) -> dsn::Polygon {
    dsn::Polygon {
        layer: zone.layer.clone(),
        width: 0.0,
        coords: zone
            .outline
            .iter()
            .map(|(x, y)| dsn::Point {
                x: *x as f64,
                y: *y as f64,
            })
            .collect(),
    }
}

/// How many pours and how many keepouts a board source holds, counted as the
/// export counts them, for the importer to check against its manifest.
pub(crate) fn zone_counts(board_source: &str) -> Result<(usize, usize)> {
    let tree = parse_sexp(board_source).context("parse KiCad board for zone inventory")?;
    let copper = copper_layers(&tree)?;
    let zones = zones(&tree, &top_level_net_table(&tree), &copper)?;
    Ok((
        zones.iter().filter(|z| z.kind == ZoneKind::Plane).count(),
        zones.iter().filter(|z| z.kind != ZoneKind::Plane).count(),
    ))
}

/// A board file name as a DSN token: whitespace, quotes and parentheses
/// become `_`, which the DSN writer would otherwise emit bare and break the
/// file (`sonde xilinx.kicad_pcb`). The SES importer accepts the same form.
pub(crate) fn dsn_design_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_whitespace() || matches!(c, '"' | '(' | ')') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// The angle `degrees` in `[0, 360)`, with float noise at whole degrees removed.
fn normalized_degrees(degrees: f64) -> f64 {
    let d = degrees.rem_euclid(360.0);
    let r = d.round();
    let d = if (d - r).abs() < 1e-9 { r } else { d };
    if d >= 360.0 {
        0.0
    } else {
        d
    }
}

/// The DSN pin ids of a footprint's copper pads, in pad order. A number used
/// once is its own id. A repeated number (two `MP` mounting pads) gets
/// `MP@1`, `MP@2`, and an unnumbered pad `@1`, `@2`, so every pin is unique
/// while the manifest keeps the KiCad number.
fn pin_ids(numbers: &[String]) -> Vec<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for number in numbers {
        *counts.entry(number.as_str()).or_default() += 1;
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    numbers
        .iter()
        .map(|number| {
            let n = seen.entry(number.as_str()).or_default();
            *n += 1;
            if counts[number.as_str()] == 1 && !number.is_empty() {
                number.clone()
            } else {
                format!("{number}@{n}")
            }
        })
        .collect()
}

/// Chord tolerance for copper graphics, in millimetres. Keepouts are
/// widened by it, so a chorded arc still covers the true copper.
const GRAPHIC_TOLERANCE_MM: f64 = 0.005;

/// A KiCad millimetre point as a DSN micrometre point (Y up).
fn dsn_point((x, y): (f64, f64)) -> dsn::Point {
    dsn::Point {
        x: (x * UM_PER_MM).round(),
        y: (-y * UM_PER_MM).round(),
    }
}

fn mm_pair(node: &SexpNode, tag: &str) -> Result<(f64, f64)> {
    xy_mm(node, tag)
}

/// The copper layers a graphic or text item is on (`*.Cu` is all of them).
fn graphic_copper_layers(node: &SexpNode, copper_layers: &[String]) -> Vec<String> {
    let mut named: Vec<&str> = node.find_str("layer").into_iter().collect();
    if let Some(list) = node.find("layers") {
        named.extend(
            list.children()
                .unwrap_or(&[])
                .iter()
                .skip(1)
                .filter_map(SexpNode::as_str),
        );
    }
    let mut out: Vec<String> = Vec::new();
    for layer in named {
        if layer == "*.Cu" {
            out.extend(copper_layers.iter().cloned());
        } else if copper_layers.iter().any(|c| c == layer) {
            out.push(layer.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn is_hidden(node: &SexpNode) -> bool {
    node.find_str("hide") == Some("yes")
        || node.find("effects").is_some_and(|e| {
            e.find_str("hide") == Some("yes")
                || e.children()
                    .unwrap_or(&[])
                    .iter()
                    .any(|c| c.as_str() == Some("hide"))
        })
}

/// The keepout shapes (one per copper layer) that cover one copper graphic or
/// text item, in the coordinates its own `(at)`/points use (board space for a
/// `gr_*`, footprint space for an `fp_*`), or `None` when it is not on copper
/// or draws nothing. Lines, arcs, curves and unfilled outlines are paths as
/// wide as their stroke plus the chord tolerance; filled shapes are polygons;
/// text is a rectangle at its own angle and justification, sized for the
/// widest glyphs (they are not measured, so it is generous, never short).
fn copper_graphic_keepouts(
    node: &SexpNode,
    copper_layers: &[String],
    parent_rotation: f64,
) -> Result<Option<Vec<dsn::Shape>>> {
    let Some(head) = node.head() else {
        return Ok(None);
    };
    let layers = graphic_copper_layers(node, copper_layers);
    if layers.is_empty() {
        return Ok(None);
    }
    let kind = head.trim_start_matches("gr_").trim_start_matches("fp_");
    let stroke = node
        .find("stroke")
        .and_then(|s| s.find_f64("width"))
        .or_else(|| node.find_f64("width"))
        .unwrap_or(0.0)
        .max(0.0);
    let filled = matches!(
        node.find("fill")
            .and_then(|f| f.get(1))
            .and_then(SexpNode::as_str),
        Some("yes" | "solid")
    ) || node.find("fill").and_then(|f| f.find_str("type")) == Some("solid");
    let width_um = ((stroke + 2.0 * GRAPHIC_TOLERANCE_MM) * UM_PER_MM).round();
    let path = |pts: Vec<(f64, f64)>| -> Vec<dsn::Shape> {
        layers
            .iter()
            .map(|layer| {
                dsn::Shape::Path(dsn::Path {
                    layer: layer.clone(),
                    width: width_um,
                    coords: pts.iter().copied().map(dsn_point).collect(),
                })
            })
            .collect()
    };
    let ring = |mut pts: Vec<(f64, f64)>| -> Vec<dsn::Shape> {
        if filled {
            return layers
                .iter()
                .map(|layer| {
                    dsn::Shape::Polygon(dsn::Polygon {
                        layer: layer.clone(),
                        width: width_um,
                        coords: pts.iter().copied().map(dsn_point).collect(),
                    })
                })
                .collect();
        }
        pts.push(pts[0]);
        path(pts)
    };
    use konnect_sexp::planar::{flatten_bezier, tessellate_arc, tessellate_circle};
    let shapes = match kind {
        "line" => path(vec![mm_pair(node, "start")?, mm_pair(node, "end")?]),
        "arc" => path(tessellate_arc(
            mm_pair(node, "start")?,
            mm_pair(node, "mid")
                .context("copper arc has no midpoint (open and save it in KiCad 10)")?,
            mm_pair(node, "end")?,
            GRAPHIC_TOLERANCE_MM,
        )),
        "circle" => {
            let (c, e) = (mm_pair(node, "center")?, mm_pair(node, "end")?);
            let r = ((e.0 - c.0).powi(2) + (e.1 - c.1).powi(2)).sqrt();
            ring(tessellate_circle(
                c,
                r + GRAPHIC_TOLERANCE_MM,
                GRAPHIC_TOLERANCE_MM,
            ))
        }
        "rect" => {
            let (a, b) = (mm_pair(node, "start")?, mm_pair(node, "end")?);
            ring(vec![a, (b.0, a.1), b, (a.0, b.1)])
        }
        "poly" => {
            let pts = node.find("pts").context("copper polygon has no points")?;
            let mut ring_pts = Vec::new();
            for item in pts.children().unwrap_or(&[]).iter().skip(1) {
                match item.head() {
                    Some("xy") => ring_pts.push((
                        finite_number(item.get_f64(1), "polygon x")?,
                        finite_number(item.get_f64(2), "polygon y")?,
                    )),
                    Some("arc") => ring_pts.extend(tessellate_arc(
                        mm_pair(item, "start")?,
                        mm_pair(item, "mid")?,
                        mm_pair(item, "end")?,
                        GRAPHIC_TOLERANCE_MM,
                    )),
                    other => bail!("copper polygon holds an unsupported '{other:?}'"),
                }
            }
            if ring_pts.len() < 3 {
                bail!("copper polygon has fewer than three points");
            }
            ring(ring_pts)
        }
        "curve" => {
            let q: Vec<(f64, f64)> = node
                .find("pts")
                .context("copper curve has no points")?
                .find_all("xy")
                .into_iter()
                .map(|xy| -> Result<(f64, f64)> {
                    Ok((
                        finite_number(xy.get_f64(1), "curve x")?,
                        finite_number(xy.get_f64(2), "curve y")?,
                    ))
                })
                .collect::<Result<_>>()?;
            if q.len() != 4 {
                bail!("copper curve does not have four control points");
            }
            path(flatten_bezier(q[0], q[1], q[2], q[3], GRAPHIC_TOLERANCE_MM))
        }
        "text" | "property" => {
            if is_hidden(node) {
                return Ok(None);
            }
            let text = match kind {
                "property" => node.get(2).and_then(SexpNode::as_str).unwrap_or(""),
                _ => node
                    .get(1)
                    .and_then(SexpNode::as_str)
                    .filter(|t| !matches!(*t, "reference" | "value" | "user"))
                    .or_else(|| node.get(2).and_then(SexpNode::as_str))
                    .unwrap_or(""),
            };
            if text.trim().is_empty() {
                return Ok(None);
            }
            let at = mm_pair(node, "at")?;
            let effects = node.find("effects");
            let font = effects.and_then(|e| e.find("font"));
            let size = font.and_then(|f| f.find("size"));
            // KiCad writes a font size as (size height width).
            let h = size.and_then(|s| s.get_f64(1)).unwrap_or(1.0).abs();
            let w = size.and_then(|s| s.get_f64(2)).unwrap_or(h).abs();
            let t = font
                .and_then(|f| f.find_f64("thickness"))
                .unwrap_or(0.15 * h)
                .abs();
            let justify: Vec<&str> = effects
                .and_then(|e| e.find("justify"))
                .map(|j| {
                    j.children()
                        .unwrap_or(&[])
                        .iter()
                        .skip(1)
                        .filter_map(SexpNode::as_str)
                        .collect()
                })
                .unwrap_or_default();
            let lines: Vec<&str> = text.split('\n').collect();
            let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
            // A glyph advance stays under 1.1 em in KiCad's fonts, even a
            // line of W; the stroke and the chord tolerance go on every side.
            let length = longest * w * 1.1;
            let pad = t + GRAPHIC_TOLERANCE_MM;
            // Along the text: its justification decides which side of the
            // anchor it runs to. Mirrored text reads the other way, so it
            // gets both sides.
            let (along_min, along_max) = if justify.contains(&"mirror") {
                (-length, length)
            } else if justify.contains(&"left") {
                (0.0, length)
            } else if justify.contains(&"right") {
                (-length, 0.0)
            } else {
                (-length / 2.0, length / 2.0)
            };
            // Across it: every line, either side of the anchor (vertical
            // justification only moves it within this).
            let across = lines.len() as f64 * h * 1.5;
            // The text angle is in board space (like a pad's); a footprint's
            // text is drawn in footprint space, so take the footprint's turn
            // off.
            let angle = node.find("at").and_then(|a| a.get_f64(3)).unwrap_or(0.0) - parent_rotation;
            let (c, sn) = (angle.to_radians().cos(), angle.to_radians().sin());
            // Unit vectors in KiCad's Y-down frame: along the text, then down
            // the page.
            let (ux, uy) = (c, -sn);
            let (vx, vy) = (sn, c);
            let corner = |a: f64, b: f64| (at.0 + a * ux + b * vx, at.1 + a * uy + b * vy);
            let corners = [
                corner(along_min - pad, -across - pad),
                corner(along_max + pad, -across - pad),
                corner(along_max + pad, across + pad),
                corner(along_min - pad, across + pad),
            ];
            layers
                .iter()
                .map(|layer| {
                    dsn::Shape::Polygon(dsn::Polygon {
                        layer: layer.clone(),
                        width: 0.0,
                        coords: corners.iter().copied().map(dsn_point).collect(),
                    })
                })
                .collect()
        }
        "text_box" => {
            let corners: Vec<(f64, f64)> = match node.find("pts") {
                Some(pts) => pts
                    .find_all("xy")
                    .into_iter()
                    .map(|xy| -> Result<(f64, f64)> {
                        Ok((
                            finite_number(xy.get_f64(1), "text box x")?,
                            finite_number(xy.get_f64(2), "text box y")?,
                        ))
                    })
                    .collect::<Result<_>>()?,
                None => {
                    let (a, b) = (mm_pair(node, "start")?, mm_pair(node, "end")?);
                    vec![a, (b.0, a.1), b, (a.0, b.1)]
                }
            };
            layers
                .iter()
                .map(|layer| {
                    dsn::Shape::Polygon(dsn::Polygon {
                        layer: layer.clone(),
                        width: width_um,
                        coords: corners.iter().copied().map(dsn_point).collect(),
                    })
                })
                .collect()
        }
        other => bail!(
            "unsupported routing profile: a '{other}' item on copper cannot be given to the router"
        ),
    };
    Ok(Some(shapes))
}

/// Board-level copper graphics and text, as structure keepouts.
fn board_copper_keepouts(tree: &SexpNode, copper_layers: &[String]) -> Result<Vec<dsn::Shape>> {
    let mut out = Vec::new();
    for node in tree.children().unwrap_or(&[]) {
        let Some(head) = node.head() else { continue };
        if !(head.starts_with("gr_") || head == "dimension" || head == "image") {
            continue;
        }
        if (head == "dimension" || head == "image")
            && !graphic_copper_layers(node, copper_layers).is_empty()
        {
            bail!("unsupported routing profile: a board '{head}' on a copper layer cannot be given to the router");
        }
        if let Some(shapes) = copper_graphic_keepouts(node, copper_layers, 0.0)
            .with_context(|| format!("board '{head}' on copper"))?
        {
            out.extend(shapes);
        }
    }
    Ok(out)
}

fn footprints(
    tree: &SexpNode,
    net_table: &BTreeMap<String, String>,
    copper_layers: &[String],
) -> Result<(Vec<FootprintModel>, usize)> {
    let mut output = Vec::new();
    let mut references = BTreeSet::new();
    let mut skipped_without_pads = 0;
    for footprint in tree.find_all("footprint") {
        // A footprint with no pads and no holes (a logo, a fiducial-free
        // graphic) has nothing for the router; leave it out.
        if footprint.find_all("pad").is_empty() {
            skipped_without_pads += 1;
            continue;
        }
        let layer = footprint
            .find_str("layer")
            .context("footprint has no layer")?;
        let kicad_side = match layer {
            "F.Cu" => "front",
            "B.Cu" => "back",
            other => bail!("unsupported routing profile: footprint on '{other}'"),
        };
        if footprint.find("clearance").is_some() {
            bail!("unsupported first routing profile: footprint has a local clearance override");
        }
        let reference = property_value(footprint, "Reference")
            .filter(|value| !value.is_empty())
            .context("footprint has no Reference property")?
            .to_string();
        if !references.insert(reference.clone()) {
            bail!("duplicate footprint reference '{reference}'");
        }
        let kiid = footprint
            .find_str("uuid")
            .context("footprint has no UUID")?
            .to_string();
        let at = footprint.find("at").context("footprint has no position")?;
        let x_um = finite_um(at.get_f64(1), "footprint x")?;
        let y_um = -finite_um(at.get_f64(2), "footprint y")?;
        // In [0, 360): Freerouting echoes `-90` as `270`, and the importer
        // checks that the router moved nothing.
        let rotation_degrees = normalized_degrees(finite_number(
            at.get_f64(3).or(Some(0.0)),
            "footprint rotation",
        )?);
        let image_name = format!("konnect_image_{reference}");

        // Footprint copper graphics and visible copper text, in footprint
        // space, as image keepouts. The file holds a back part's graphics
        // already mirrored, like its pads.
        let mut copper_keepouts = Vec::new();
        for node in footprint.children().unwrap_or(&[]) {
            let Some(head) = node.head() else { continue };
            if !(head.starts_with("fp_") || head == "property") {
                continue;
            }
            if let Some(shapes) = copper_graphic_keepouts(node, copper_layers, rotation_degrees)
                .with_context(|| format!("footprint '{reference}' '{head}' on copper"))?
            {
                copper_keepouts.extend(shapes);
            }
        }
        let mut pads = Vec::new();
        let mut holes = Vec::new();
        for pad in footprint.find_all("pad") {
            let number = pad
                .get(1)
                .and_then(SexpNode::as_str)
                .context("pad has no number field")?
                .to_string();
            let label = if number.is_empty() {
                format!("{reference} (unnumbered pad)")
            } else {
                format!("{reference}-{number}")
            };
            let pad_type = pad
                .get(2)
                .and_then(SexpNode::as_str)
                .context("pad has no type")?;
            if !matches!(pad_type, "smd" | "connect" | "thru_hole" | "np_thru_hole") {
                bail!("unsupported routing profile: pad {label} has type '{pad_type}'");
            }
            let at = pad.find("at").context("pad has no position")?;
            let px_um = finite_um(at.get_f64(1), "pad x")?;
            let py_um = -finite_um(at.get_f64(2), "pad y")?;
            // KiCad stores a pad's angle in board space, the footprint's
            // rotation included; the DSN wants it relative to the image.
            let pad_angle = finite_number(at.get_f64(3).or(Some(0.0)), "pad rotation")?;
            let relative = normalized_degrees(pad_angle - rotation_degrees);
            let size = pad.find("size").context("pad has no size")?;
            let size_x_um = positive_um(size.get_f64(1), "pad width")?;
            let size_y_um = positive_um(size.get_f64(2), "pad height")?;
            let drill = pad.find("drill");
            // An oval drill is `(drill oval w h)`, a round one `(drill d)`.
            let drill_dims = match drill {
                None => None,
                Some(d) if d.get(1).and_then(SexpNode::as_str) == Some("oval") => Some((
                    positive_um(d.get_f64(2), "pad drill width")?,
                    positive_um(d.get_f64(3).or(d.get_f64(2)), "pad drill height")?,
                )),
                Some(d) => {
                    let v = positive_um(d.get_f64(1), "pad drill")?;
                    Some((v, v))
                }
            };

            if pad_type == "np_thru_hole" {
                // An unplated hole: no pin, an obstacle on every copper layer,
                // as large as the larger of its pad and its drill.
                let (dx, dy) = drill_dims.unwrap_or((size_x_um, size_y_um));
                holes.push(HoleModel {
                    x_um: px_um,
                    y_um: py_um,
                    rotation_degrees: relative,
                    size_x_um: size_x_um.max(dx),
                    size_y_um: size_y_um.max(dy),
                });
                continue;
            }

            let shape = match pad.get(3).and_then(SexpNode::as_str) {
                Some("circle") => PadShape::Circle,
                Some("rect") => PadShape::Rect,
                Some("roundrect") => PadShape::RoundRect,
                Some("oval") => PadShape::Oval,
                Some(other) => {
                    bail!("unsupported routing profile: pad {label} has shape '{other}'")
                }
                None => bail!("pad {label} has no shape"),
            };
            if shape == PadShape::Circle && size_x_um != size_y_um {
                bail!("circle pad {label} does not have equal X/Y size");
            }
            // An oval of equal sides is a circle.
            let shape = if shape == PadShape::Oval && size_x_um == size_y_um {
                PadShape::Circle
            } else {
                shape
            };
            if pad.find("clearance").is_some() {
                bail!(
                    "unsupported first routing profile: pad {label} has a local clearance override"
                );
            }
            if pad.find_str("remove_unused_layers") == Some("yes") {
                bail!("unsupported first routing profile: pad {label} removes copper on unused layers");
            }
            // KiCad writes a pad's shape offset inside its drill. Before PR 16
            // the exporter looked for it on the pad, never found it, and put
            // offset copper (a TO-92's pads) on its hole instead.
            if let Some(offset) = pad.find("offset") {
                // Not where KiCad writes it: refuse rather than guess its frame.
                let x = finite_um(offset.get_f64(1), "pad offset x")?;
                let y = finite_um(offset.get_f64(2), "pad offset y")?;
                if x != 0 || y != 0 {
                    bail!("unsupported first routing profile: pad {label} has a non-zero shape offset outside its drill");
                }
            }
            let (offset_x_um, offset_y_um) = match drill.and_then(|d| d.find("offset")) {
                Some(offset) => (
                    finite_um(offset.get_f64(1), "pad offset x")?,
                    -finite_um(offset.get_f64(2), "pad offset y")?,
                ),
                None => (0, 0),
            };
            let layers = pad
                .find("layers")
                .and_then(SexpNode::children)
                .unwrap_or(&[])
                .iter()
                .skip(1)
                .filter_map(SexpNode::as_str)
                .filter(|layer| layer.ends_with(".Cu") || *layer == "*.Cu")
                .map(str::to_string)
                .collect::<Vec<_>>();
            // A pad with no copper (a paste or mask aperture) is nothing the
            // router can touch.
            if layers.is_empty() {
                continue;
            }
            let net = pad
                .find("net")
                .and_then(|node| resolve_net(node, net_table));
            // A plated hole with no ring and no net (a plated mounting hole
            // drilled to its pad size) is a hole to the router, like an
            // unplated one.
            if pad_type == "thru_hole" && net.is_none() {
                if let Some((dx, dy)) = drill_dims {
                    if dx >= size_x_um && dy >= size_y_um {
                        holes.push(HoleModel {
                            x_um: px_um,
                            y_um: py_um,
                            rotation_degrees: relative,
                            size_x_um: size_x_um.max(dx),
                            size_y_um: size_y_um.max(dy),
                        });
                        continue;
                    }
                }
            }
            let (layers, drill_um) = if pad_type == "smd" || pad_type == "connect" {
                // One outer copper layer, either side: the DSN never mirrors,
                // so a pad's own layer is the layer the router sees.
                if layers != ["F.Cu".to_string()] && layers != ["B.Cu".to_string()] {
                    bail!(
                        "unsupported SMD pad {label}: copper layers are [{}], not one outer layer",
                        layers.join(", ")
                    );
                }
                (layers, None)
            } else {
                // A plated hole has copper on every layer of the stack (pads
                // that remove unused layers are refused above).
                if !(layers == ["*.Cu".to_string()]
                    || layers == ["F.Cu".to_string(), "B.Cu".to_string()])
                {
                    bail!(
                        "unsupported through-hole pad {label}: copper layers are [{}]",
                        layers.join(", ")
                    );
                }
                let (dx, dy) = drill_dims.context("through-hole pad has no drill")?;
                // The router sees the copper; a slot only has to fit inside it.
                if dx >= size_x_um || dy >= size_y_um {
                    bail!("pad {label} drill {dx}x{dy} um is not smaller than its copper size");
                }
                (copper_layers.to_vec(), Some(dx.min(dy)))
            };
            pads.push(PadModel {
                number,
                dsn_id: String::new(),
                x_um: px_um,
                y_um: py_um,
                rotation_degrees: relative,
                net,
                padstack: PadstackKey {
                    shape,
                    layers,
                    size_x_um,
                    size_y_um,
                    drill_um,
                    offset_x_um,
                    offset_y_um,
                },
            });
        }
        // Stable order: by KiCad number, then position, so repeated numbers
        // get their `@n` the same way on every export.
        pads.sort_by(|left, right| {
            (&left.number, left.x_um, left.y_um).cmp(&(&right.number, right.x_um, right.y_um))
        });
        let ids = pin_ids(&pads.iter().map(|p| p.number.clone()).collect::<Vec<_>>());
        for (pad, id) in pads.iter_mut().zip(ids) {
            pad.dsn_id = id;
        }
        holes.sort_by_key(|hole| (hole.x_um, hole.y_um));
        output.push(FootprintModel {
            reference,
            kiid,
            image_name,
            x_um,
            y_um,
            rotation_degrees,
            kicad_side,
            pads,
            holes,
            copper_keepouts,
        });
    }
    output.sort_by(|left, right| left.reference.cmp(&right.reference));
    Ok((output, skipped_without_pads))
}

fn normalize_rules(
    net_pins: &BTreeMap<String, Vec<String>>,
    effective_rules: &IpcEffectiveRoutingRules,
) -> Result<(RuleGroups, NetClassNames)> {
    let mut class_nets: BTreeMap<RuleKey, Vec<String>> = BTreeMap::new();
    for net in net_pins.keys() {
        let rules = effective_rules.get(net).with_context(|| {
            format!("KiCad returned no effective routing rules for net '{net}'")
        })?;
        let key = RuleKey {
            track_width_um: positive_um(rules.track_width_mm, "track width")?,
            clearance_um: non_negative_um(rules.clearance_mm, "clearance")?,
            via_diameter_um: positive_um(rules.via_diameter_mm, "via diameter")?,
            via_drill_um: positive_um(rules.via_drill_mm, "via drill")?,
        };
        if key.via_drill_um >= key.via_diameter_um {
            bail!(
                "net '{net}' has via drill {} um not smaller than diameter {} um",
                key.via_drill_um,
                key.via_diameter_um
            );
        }
        class_nets.entry(key).or_default().push(net.clone());
    }
    for nets in class_nets.values_mut() {
        nets.sort();
    }
    let mut net_classes = BTreeMap::new();
    for (index, nets) in class_nets.values().enumerate() {
        let name = format!("konnect_class_{:04}", index + 1);
        for net in nets {
            net_classes.insert(net.clone(), name.clone());
        }
    }
    Ok((class_nets, net_classes))
}

#[allow(clippy::too_many_arguments)]
fn build_pcb(
    board_path: &Path,
    copper_stack: &[(String, &'static str)],
    outline: &[(i64, i64)],
    footprints: &[FootprintModel],
    net_pins: &BTreeMap<String, Vec<String>>,
    class_nets: &BTreeMap<RuleKey, Vec<String>>,
    net_classes: &BTreeMap<String, String>,
    padstack_names: &BTreeMap<PadstackKey, String>,
    via_names: &BTreeMap<RuleKey, String>,
    locked_routing: &LockedRouting,
    zones: &[ZoneModel],
    board_keepouts: &[dsn::Shape],
) -> Result<dsn::Pcb> {
    let default_rule = class_nets
        .keys()
        .next()
        .context("routing model has no rule class")?;
    let copper_layers: Vec<String> = copper_stack.iter().map(|(name, _)| name.clone()).collect();
    let layers = copper_stack
        .iter()
        .enumerate()
        .map(|(index, (name, kind))| dsn::Layer {
            name: name.clone(),
            r#type: kind.to_string(),
            property: Some(dsn::Property { index }),
        })
        .collect();
    let boundary = dsn::Boundary::Path(dsn::Path {
        layer: "pcb".to_string(),
        width: 0.0,
        coords: outline
            .iter()
            .map(|(x, y)| dsn::Point {
                x: *x as f64,
                y: *y as f64,
            })
            .collect(),
    });

    let components = footprints
        .iter()
        .map(|footprint| dsn::Component {
            name: footprint.image_name.clone(),
            places: vec![dsn::Place {
                name: footprint.reference.clone(),
                x: footprint.x_um as f64,
                y: footprint.y_um as f64,
                side: "front".to_string(),
                rotation: footprint.rotation_degrees,
                PN: None,
            }],
        })
        .collect();
    let images = footprints
        .iter()
        .map(|footprint| dsn::Image {
            name: footprint.image_name.clone(),
            outlines: Vec::new(),
            pins: footprint
                .pads
                .iter()
                .map(|pad| dsn::Pin {
                    name: padstack_names[&pad.padstack].clone(),
                    rotate: (pad.rotation_degrees != 0.0).then_some(pad.rotation_degrees),
                    id: pad.dsn_id.clone(),
                    x: pad.x_um as f64,
                    y: pad.y_um as f64,
                })
                .collect(),
            keepouts: dsn::Keepouts(
                footprint
                    .holes
                    .iter()
                    .flat_map(|hole| {
                        copper_layers
                            .iter()
                            .map(move |layer| hole_shape(hole, layer))
                    })
                    // Already in image coordinates: footprint space with Y up,
                    // the frame the pins use.
                    .chain(footprint.copper_keepouts.iter().cloned())
                    .map(|shape| dsn::Keepout {
                        kind: dsn::KeepoutKind::Normal,
                        id_: String::new(),
                        sequence_number: None,
                        shape,
                        rule: None,
                    })
                    .collect(),
            ),
        })
        .collect();

    let mut padstacks = padstack_names
        .iter()
        .map(|(key, name)| padstack(name, key))
        .collect::<Vec<_>>();
    padstacks.extend(via_names.iter().map(|(rule, name)| {
        dsn::Padstack {
            name: name.clone(),
            shapes: copper_layers
                .iter()
                .map(|layer| {
                    dsn::Shape::Circle(dsn::Circle {
                        layer: layer.clone(),
                        diameter: rule.via_diameter_um as f64,
                        offset: None,
                    })
                })
                .collect(),
            attach: Some(false),
        }
    }));
    padstacks.sort_by(|left, right| left.name.cmp(&right.name));

    let nets = net_pins
        .iter()
        .map(|(name, pins)| dsn::NetPinAssignments {
            name: name.clone(),
            pins: Some(dsn::Pins {
                names: pins.clone(),
            }),
        })
        .collect();
    let classes = class_nets
        .iter()
        .enumerate()
        .map(|(index, (rule, nets))| dsn::Class {
            name: format!("konnect_class_{:04}", index + 1),
            nets: nets.clone(),
            circuit: dsn::Circuit {
                use_via: via_names[rule].clone(),
            },
            rule: dsn::Rule {
                width: rule.track_width_um as f32,
                clearances: vec![dsn::Clearance {
                    value: rule.clearance_um as f32,
                    r#type: None,
                }],
            },
        })
        .collect();
    debug_assert_eq!(net_classes.len(), net_pins.len());

    let net_rules = class_nets
        .iter()
        .flat_map(|(rule, nets)| nets.iter().map(move |net| (net, rule)))
        .collect::<BTreeMap<_, _>>();
    let wires = locked_routing
        .tracks
        .iter()
        .map(|track| {
            if !net_rules.contains_key(&track.net) {
                bail!("locked track uses unknown routing net '{}'", track.net);
            }
            Ok(dsn::Wire {
                path: dsn::Path {
                    layer: track.layer.clone(),
                    width: track.width_um as f64,
                    coords: vec![
                        dsn::Point {
                            x: track.start_x_um as f64,
                            y: track.start_y_um as f64,
                        },
                        dsn::Point {
                            x: track.end_x_um as f64,
                            y: track.end_y_um as f64,
                        },
                    ],
                },
                net: track.net.clone(),
                r#type: "fix".to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let fixed_vias = locked_routing
        .vias
        .iter()
        .map(|via| {
            let rule = net_rules
                .get(&via.net)
                .with_context(|| format!("locked via uses unknown routing net '{}'", via.net))?;
            if via.diameter_um != rule.via_diameter_um || via.drill_um != rule.via_drill_um {
                bail!(
                    "locked via on net '{}' is {}:{} um, but its effective via rule is {}:{} um",
                    via.net,
                    via.diameter_um,
                    via.drill_um,
                    rule.via_diameter_um,
                    rule.via_drill_um
                );
            }
            Ok(dsn::Via {
                name: via_names[*rule].clone(),
                x: via.x_um as f64,
                y: via.y_um as f64,
                net: via.net.clone(),
                r#type: Some("fix".to_string()),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(dsn::Pcb {
        name: dsn_design_name(
            board_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("board.kicad_pcb"),
        ),
        parser: Some(dsn::Parser {
            string_quote: Some('"'),
            space_in_quoted_tokens: Some(true),
            host_cad: Some("Konnect".to_string()),
            host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
        resolution: dsn::Resolution {
            unit: "um".to_string(),
            value: DSN_RESOLUTION,
        },
        unit: Some("um".to_string()),
        structure: dsn::Structure {
            layers,
            boundary,
            place_boundary: None,
            planes: zones
                .iter()
                .filter(|zone| zone.kind == ZoneKind::Plane)
                .map(|zone| dsn::Plane {
                    net: zone.net.clone().unwrap_or_default(),
                    polygon: zone_polygon(zone),
                })
                .collect(),
            keepouts: dsn::Keepouts(
                zones
                    .iter()
                    .filter(|zone| zone.kind != ZoneKind::Plane)
                    .map(|zone| dsn::Keepout {
                        kind: match zone.kind {
                            ZoneKind::KeepoutTracks => dsn::KeepoutKind::Wire,
                            ZoneKind::KeepoutVias => dsn::KeepoutKind::Via,
                            _ => dsn::KeepoutKind::Normal,
                        },
                        id_: String::new(),
                        sequence_number: None,
                        shape: dsn::Shape::Polygon(zone_polygon(zone)),
                        rule: None,
                    })
                    // Copper graphics and text: nothing may cross them.
                    .chain(board_keepouts.iter().cloned().map(|shape| dsn::Keepout {
                        kind: dsn::KeepoutKind::Normal,
                        id_: String::new(),
                        sequence_number: None,
                        shape,
                        rule: None,
                    }))
                    .collect(),
            ),
            via: dsn::ViaNames {
                names: via_names.values().cloned().collect(),
            },
            grids: Vec::new(),
            rules: vec![dsn::StructureRule {
                width: Some(default_rule.track_width_um as f32),
                clearances: vec![dsn::Clearance {
                    value: default_rule.clearance_um as f32,
                    r#type: None,
                }],
            }],
        },
        placement: dsn::Placement { components },
        library: dsn::Library { images, padstacks },
        network: dsn::Network { nets, classes },
        wiring: dsn::Wiring {
            wires,
            vias: fixed_vias,
        },
    })
}

/// An unplated hole as a keepout on `layer`: a circle, or for a slot the
/// stadium of its long axis, turned by the hole's angle in the image.
fn hole_shape(hole: &HoleModel, layer: &str) -> dsn::Shape {
    let (cx, cy) = (hole.x_um as f64, hole.y_um as f64);
    if hole.size_x_um == hole.size_y_um {
        return dsn::Shape::Circle(dsn::Circle {
            layer: layer.to_string(),
            diameter: hole.size_x_um as f64,
            offset: Some(dsn::Point { x: cx, y: cy }),
        });
    }
    let half = (hole.size_x_um - hole.size_y_um).abs() as f64 / 2.0;
    let (ux, uy) = if hole.size_x_um >= hole.size_y_um {
        (half, 0.0)
    } else {
        (0.0, half)
    };
    let t = hole.rotation_degrees.to_radians();
    let (rx, ry) = (ux * t.cos() - uy * t.sin(), ux * t.sin() + uy * t.cos());
    dsn::Shape::Path(dsn::Path {
        layer: layer.to_string(),
        width: hole.size_x_um.min(hole.size_y_um) as f64,
        coords: vec![
            dsn::Point {
                x: cx - rx,
                y: cy - ry,
            },
            dsn::Point {
                x: cx + rx,
                y: cy + ry,
            },
        ],
    })
}

fn padstack(name: &str, key: &PadstackKey) -> dsn::Padstack {
    let (ox, oy) = (key.offset_x_um as f64, key.offset_y_um as f64);
    let shapes = key
        .layers
        .iter()
        .map(|layer| match key.shape {
            PadShape::Circle => dsn::Shape::Circle(dsn::Circle {
                layer: layer.clone(),
                diameter: key.size_x_um as f64,
                offset: (ox != 0.0 || oy != 0.0).then_some(dsn::Point { x: ox, y: oy }),
            }),
            // A rounded rectangle is written as its bounding rectangle: the router
            // may not use the corners, and never sees copper it does not have.
            PadShape::Rect | PadShape::RoundRect => dsn::Shape::Rect(dsn::Rect {
                layer: layer.clone(),
                x1: ox - (key.size_x_um as f64) / 2.0,
                y1: oy - (key.size_y_um as f64) / 2.0,
                x2: ox + key.size_x_um as f64 / 2.0,
                y2: oy + key.size_y_um as f64 / 2.0,
            }),
            // A stadium: a path along the long axis as wide as the short one.
            PadShape::Oval => {
                let half = (key.size_x_um - key.size_y_um).abs() as f64 / 2.0;
                let (x, y) = if key.size_x_um >= key.size_y_um {
                    (half, 0.0)
                } else {
                    (0.0, half)
                };
                dsn::Shape::Path(dsn::Path {
                    layer: layer.clone(),
                    width: key.size_x_um.min(key.size_y_um) as f64,
                    coords: vec![
                        dsn::Point {
                            x: ox - x,
                            y: oy - y,
                        },
                        dsn::Point {
                            x: ox + x,
                            y: oy + y,
                        },
                    ],
                })
            }
        })
        .collect();
    dsn::Padstack {
        name: name.to_string(),
        shapes,
        attach: Some(false),
    }
}

fn serialize_and_validate(pcb: dsn::Pcb) -> Result<String> {
    let file = dsn::DsnFile { pcb };
    let mut bytes = Vec::new();
    {
        let mut writer = ListWriter::new(&mut bytes);
        file.write_dsn(&mut writer)
            .context("serialize Specctra DSN")?;
    }
    let mut text = String::from_utf8(bytes).context("Specctra writer emitted non-UTF-8")?;
    if text.starts_with('\n') {
        text.remove(0);
    }
    text.push('\n');

    let cursor = Cursor::new(text.as_bytes());
    let mut tokenizer = ListTokenizer::new(BufReader::new(cursor));
    dsn::DsnFile::read_dsn(&mut tokenizer)
        .map_err(|error| anyhow::anyhow!("generated DSN failed parser round-trip: {error}"))?;
    Ok(text)
}

#[allow(clippy::too_many_arguments)]
fn build_manifest(
    board_path: &Path,
    source_sha256: &str,
    pours_omitted: bool,
    skipped_without_pads: usize,
    board_keepout_count: usize,
    copper_stack: &[(String, &'static str)],
    footprints: &[FootprintModel],
    net_pins: &BTreeMap<String, Vec<String>>,
    net_classes: &BTreeMap<String, String>,
    padstack_names: &BTreeMap<PadstackKey, String>,
    via_names: &BTreeMap<RuleKey, String>,
    locked_routing: &LockedRouting,
    zones: &[ZoneModel],
) -> Result<String> {
    let components = footprints
        .iter()
        .map(|footprint| ManifestComponent {
            reference: footprint.reference.clone(),
            kiid: footprint.kiid.clone(),
            image_name: footprint.image_name.clone(),
            x_um: footprint.x_um,
            y_um: footprint.y_um,
            rotation_degrees: footprint.rotation_degrees,
            side: "front".to_string(),
            kicad_side: footprint.kicad_side.to_string(),
            pads: footprint
                .pads
                .iter()
                .map(|pad| ManifestPin {
                    pad_number: pad.number.clone(),
                    dsn_pin: format!("{}-{}", footprint.reference, pad.dsn_id),
                    net: pad.net.clone(),
                    padstack_name: padstack_names[&pad.padstack].clone(),
                })
                .collect(),
        })
        .collect();
    let nets = net_pins
        .iter()
        .map(|(name, pins)| ManifestNet {
            name: name.clone(),
            pins: pins.clone(),
            class_name: net_classes[name].clone(),
        })
        .collect();
    let mut padstacks = padstack_names
        .iter()
        .map(|(key, name)| ManifestPadstack {
            name: name.clone(),
            purpose: "pad".to_string(),
            shape: key.shape,
            layers: key.layers.clone(),
            size_x_um: key.size_x_um,
            size_y_um: key.size_y_um,
            drill_um: key.drill_um,
        })
        .collect::<Vec<_>>();
    let copper_layers: Vec<String> = copper_stack.iter().map(|(name, _)| name.clone()).collect();
    let any_back = footprints.iter().any(|f| f.kicad_side == "back");
    padstacks.extend(via_names.iter().map(|(rule, name)| ManifestPadstack {
        name: name.clone(),
        purpose: "via".to_string(),
        shape: PadShape::Circle,
        layers: copper_layers.clone(),
        size_x_um: rule.via_diameter_um,
        size_y_um: rule.via_diameter_um,
        drill_um: Some(rule.via_drill_um),
    }));
    padstacks.sort_by(|left, right| left.name.cmp(&right.name));

    serde_json::to_string_pretty(&Manifest {
        routing_context: None,
        schema_version: 1,
        board_path: board_path.display().to_string(),
        source_sha256: source_sha256.to_string(),
        coordinate_unit: "um".to_string(),
        resolution: DSN_RESOLUTION as u32,
        supported_profile: SupportedProfile {
            copper_layers: copper_layers.len() as u32,
            component_side: if any_back { "front_and_back" } else { "front" }.to_string(),
            pad_shapes: ["circle", "rect", "roundrect", "oval"]
                .map(str::to_string)
                .to_vec(),
            existing_routing: !locked_routing.tracks.is_empty() || !locked_routing.vias.is_empty(),
            locked_track_count: locked_routing.tracks.len(),
            locked_via_count: locked_routing.vias.len(),
            copper_zones: !zones.is_empty(),
            zone_plane_count: zones.iter().filter(|z| z.kind == ZoneKind::Plane).count(),
            zone_keepout_count: zones.iter().filter(|z| z.kind != ZoneKind::Plane).count(),
            custom_rules: false,
            footprints_without_pads: skipped_without_pads,
            pours_omitted,
            copper_graphic_keepout_count: board_keepout_count
                + footprints
                    .iter()
                    .map(|f| f.copper_keepouts.len())
                    .sum::<usize>(),
            outline: "one closed loop of Edge.Cuts lines, arcs, a rectangle, a circle or a \
                      polygon, with arcs and circles as chords within 5 um"
                .to_string(),
        },
        layers: copper_stack
            .iter()
            .enumerate()
            .map(|(index, (layer, kind))| ManifestLayer {
                kicad_name: layer.clone(),
                dsn_name: layer.clone(),
                index,
                dsn_type: kind.to_string(),
            })
            .collect(),
        components,
        nets,
        padstacks,
    })
    .context("serialize routing manifest")
}

fn top_level_net_table(tree: &SexpNode) -> BTreeMap<String, String> {
    tree.find_all("net")
        .into_iter()
        .filter_map(|net| {
            let id = net.get(1)?.as_str()?;
            let name = net.get(2)?.as_str()?;
            Some((id.to_string(), name.to_string()))
        })
        .collect()
}

fn resolve_net(net: &SexpNode, table: &BTreeMap<String, String>) -> Option<String> {
    if let Some(name) = net.get(2).and_then(SexpNode::as_str) {
        return (!name.is_empty()).then(|| name.to_string());
    }
    let value = net.get(1)?.as_str()?;
    if let Some(name) = table.get(value).filter(|name| !name.is_empty()) {
        return Some(name.clone());
    }
    // KiCad 10 stores the net name directly as `(net "NAME")`. Older board
    // files used `(net id "NAME")`, handled by the branch above.
    (!value.is_empty()).then(|| value.to_string())
}

fn property_value<'a>(node: &'a SexpNode, property_name: &str) -> Option<&'a str> {
    node.find_all("property")
        .into_iter()
        .find(|property| property.get(1).and_then(SexpNode::as_str) == Some(property_name))?
        .get(2)?
        .as_str()
}

fn point_um(node: &SexpNode, tag: &str) -> Result<(i64, i64)> {
    let point = node.find(tag).with_context(|| format!("missing '{tag}'"))?;
    Ok((
        finite_um(point.get_f64(1), tag)?,
        -finite_um(point.get_f64(2), tag)?,
    ))
}

fn finite_number(value: Option<f64>, label: &str) -> Result<f64> {
    let value = value.with_context(|| format!("missing {label}"))?;
    if !value.is_finite() {
        bail!("{label} is not finite");
    }
    Ok(value)
}

fn finite_um(value: Option<f64>, label: &str) -> Result<i64> {
    let value = finite_number(value, label)? * UM_PER_MM;
    if value < i64::MIN as f64 || value > i64::MAX as f64 {
        bail!("{label} is outside the supported coordinate range");
    }
    Ok(value.round() as i64)
}

fn positive_um(value: Option<f64>, label: &str) -> Result<i64> {
    let value = finite_um(value, label)?;
    if value <= 0 {
        bail!("{label} must be greater than zero");
    }
    Ok(value)
}

fn non_negative_um(value: Option<f64>, label: &str) -> Result<i64> {
    let value = finite_um(value, label)?;
    if value < 0 {
        bail!("{label} must not be negative");
    }
    Ok(value)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnect_ipc::IpcRoutingRules;

    fn rules() -> IpcEffectiveRoutingRules {
        ["GND", "VCC"]
            .into_iter()
            .map(|net| {
                (
                    net.to_string(),
                    IpcRoutingRules {
                        class_name: "Default".to_string(),
                        constituents: vec!["Default".to_string()],
                        track_width_mm: Some(0.25),
                        clearance_mm: Some(0.2),
                        via_diameter_mm: Some(0.6),
                        via_drill_mm: Some(0.3),
                    },
                )
            })
            .collect()
    }

    fn native_fixture_rules() -> IpcEffectiveRoutingRules {
        let mut rules = rules();
        for rule in rules.values_mut() {
            rule.track_width_mm = Some(0.2);
        }
        rules
    }

    #[test]
    fn deterministic_export_round_trips_through_specctra_parser() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let first = export_dsn(Path::new("board.kicad_pcb"), source, &rules()).unwrap();
        let second = export_dsn(Path::new("board.kicad_pcb"), source, &rules()).unwrap();

        assert_eq!(first.dsn, second.dsn);
        assert_eq!(first.manifest, second.manifest);
        assert_eq!(first.component_count, 2);
        assert_eq!(first.pad_count, 4);
        assert_eq!(first.net_count, 2);
        assert!(first.dsn.contains("(pcb board.kicad_pcb"));
        assert!(first.dsn.contains("(boundary"));
        assert!(first.dsn.contains("(net GND"));
        assert!(first.dsn.contains("R1-1"));
        let manifest: serde_json::Value = serde_json::from_str(&first.manifest).unwrap();
        assert_eq!(manifest["components"][0]["reference"], "R1");
        assert_eq!(manifest["components"][0]["x_um"], 100_000);
        assert_eq!(manifest["components"][0]["y_um"], -50_000);
        assert_eq!(manifest["components"][0]["rotation_degrees"], 0.0);
        assert_eq!(manifest["components"][0]["side"], "front");
    }

    #[test]
    fn freerouting_owned_dsn_corpus_parses() {
        let source = include_str!("../tests/fixtures/freerouting_issue269_minimal_v2_3_0.dsn");
        let mut tokenizer = ListTokenizer::new(BufReader::new(Cursor::new(source.as_bytes())));
        dsn::DsnFile::read_dsn(&mut tokenizer)
            .expect("Freerouting v2.3.0 corpus fixture must remain parseable");
    }

    #[test]
    fn native_kicad_dsn_rewrites_manifest_identifiers_without_changing_semantics() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let native = include_str!("../tests/fixtures/specctra_two_resistors.native-kicad-10.dsn");
        let baseline = export_dsn(
            Path::new("board.kicad_pcb"),
            source,
            &native_fixture_rules(),
        )
        .unwrap();
        let adopted = adopt_native_dsn(baseline, native.to_string()).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&adopted.manifest).unwrap();

        assert_eq!(adopted.dsn, native);
        assert_eq!(
            manifest["components"][0]["image_name"],
            "Resistor_SMD:R_0402"
        );
        assert_eq!(
            manifest["components"][0]["pads"][0]["padstack_name"],
            "Rect[T]Pad_600.000000x500.000000_um"
        );
        assert_eq!(manifest["nets"][0]["class_name"], "kicad_default");
        assert!(manifest["padstacks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|padstack| padstack["name"] == "Via[0-1]_600:300_um"));
    }

    /// Optional local parity check against the Freerouting engine. CI does not
    /// install Java or Freerouting; maintainers can opt in with
    /// `FREEROUTING_JAR=/path/to/freerouting.jar cargo test -p konnect-core
    /// freerouting_accepts_exported_fixture -- --ignored`.
    #[test]
    #[ignore = "requires Java and FREEROUTING_JAR"]
    fn freerouting_accepts_exported_fixture() {
        let jar = std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR");
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let temp = tempfile::tempdir().expect("tempdir");
        let board_path = temp.path().join("board.kicad_pcb");
        let dsn_path = temp.path().join("board.dsn");
        let ses_path = temp.path().join("board.ses");
        std::fs::write(&board_path, source).expect("write board fixture");
        let export = export_dsn(&board_path, source, &rules()).unwrap();
        std::fs::write(&dsn_path, export.dsn).expect("write DSN");

        let output = std::process::Command::new("java")
            .arg("-jar")
            .arg(jar)
            .arg("-de")
            .arg(&dsn_path)
            .arg("-do")
            .arg(&ses_path)
            .arg("-mp")
            .arg("2")
            .output()
            .expect("launch Freerouting");

        assert!(
            output.status.success(),
            "Freerouting refused generated DSN:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            std::fs::metadata(&ses_path)
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false),
            "Freerouting did not produce a non-empty SES"
        );
        let ses = std::fs::read_to_string(&ses_path).expect("read Freerouting SES");
        crate::specctra_ses::parse_import_plan(&board_path, source, &export.manifest, &ses)
            .expect("Freerouting SES must pass Konnect's strict import planner");
    }

    /// Real-engine parity for KiCad 10's native exporter identifiers plus the
    /// rewritten revision manifest consumed by the strict SES planner.
    #[test]
    #[ignore = "requires Java and FREEROUTING_JAR"]
    fn freerouting_round_trips_native_kicad_export() {
        let jar = std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR");
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let native = include_str!("../tests/fixtures/specctra_two_resistors.native-kicad-10.dsn");
        let temp = tempfile::tempdir().expect("tempdir");
        let board_path = temp.path().join("board.kicad_pcb");
        let dsn_path = temp.path().join("board.dsn");
        let ses_path = temp.path().join("board.ses");
        std::fs::write(&board_path, source).expect("write board fixture");
        let baseline = export_dsn(&board_path, source, &native_fixture_rules()).unwrap();
        let export = adopt_native_dsn(baseline, native.to_string()).unwrap();
        std::fs::write(&dsn_path, &export.dsn).expect("write native DSN");

        let output = std::process::Command::new("java")
            .arg("-jar")
            .arg(jar)
            .arg("-de")
            .arg(&dsn_path)
            .arg("-do")
            .arg(&ses_path)
            .arg("-mp")
            .arg("2")
            .output()
            .expect("run Freerouting");
        assert!(
            output.status.success(),
            "Freerouting failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let ses = std::fs::read_to_string(&ses_path).expect("read SES");
        let plan =
            crate::specctra_ses::parse_import_plan(&board_path, source, &export.manifest, &ses)
                .expect("strict SES planner accepts native KiCad identifiers");
        assert!(!plan.tracks.is_empty() || !plan.vias.is_empty());
    }

    #[test]
    fn incomplete_effective_rules_are_refused() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb");
        let mut rules = rules();
        rules.get_mut("GND").unwrap().via_drill_mm = None;
        let error = export_dsn(Path::new("board.kicad_pcb"), source, &rules)
            .unwrap_err()
            .to_string();
        assert!(error.contains("via drill"), "{error}");
    }

    #[test]
    fn unlocked_routing_is_refused_before_export() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb").replace(
            "\n)",
            "\n  (segment (start 1 1) (end 2 2) (width 0.2) (layer \"F.Cu\") (net 1))\n)",
        );
        let error = export_dsn(Path::new("board.kicad_pcb"), &source, &rules())
            .unwrap_err()
            .to_string();
        assert!(error.contains("existing track segment"), "{error}");
    }

    #[test]
    fn locked_tracks_and_vias_are_exported_as_fixed_wiring() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors_locked.kicad_pcb");
        let export = export_dsn(Path::new("board.kicad_pcb"), source, &rules()).unwrap();
        assert!(export.dsn.contains("(type fix)"));
        assert!(export.dsn.contains("(path F.Cu 250"));
        assert!(export.dsn.contains("(via konnect_via_"));
        let manifest: serde_json::Value = serde_json::from_str(&export.manifest).unwrap();
        assert_eq!(manifest["supported_profile"]["existing_routing"], true);
        assert_eq!(manifest["supported_profile"]["locked_track_count"], 1);
        assert_eq!(manifest["supported_profile"]["locked_via_count"], 1);
    }

    #[test]
    fn freerouting_session_preserves_locked_routing_outside_the_import_plan() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors_locked.kicad_pcb");
        let native =
            include_str!("../tests/fixtures/specctra_two_resistors_locked.native-kicad-10.dsn");
        let ses =
            include_str!("../tests/fixtures/specctra_two_resistors_locked.freerouting-2.3.0.ses");
        let temp = tempfile::tempdir().unwrap();
        let board_path = temp.path().join("specctra_two_resistors_locked.kicad_pcb");
        std::fs::write(&board_path, source).unwrap();
        let baseline = export_dsn(&board_path, source, &native_fixture_rules()).unwrap();
        let adopted = adopt_native_dsn(baseline, native.to_string()).unwrap();
        let plan =
            crate::specctra_ses::parse_import_plan(&board_path, source, &adopted.manifest, ses)
                .unwrap();
        assert_eq!(plan.locked_track_count, 1);
        assert_eq!(plan.locked_via_count, 1);
        assert!(!plan.tracks.is_empty() || !plan.vias.is_empty());
    }

    #[test]
    fn locked_arcs_are_refused_instead_of_approximated() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors_locked_arc.kicad_pcb");
        let error = export_dsn(Path::new("board.kicad_pcb"), source, &rules())
            .unwrap_err()
            .to_string();
        assert!(error.contains("locked routed arc"), "{error}");
        assert!(error.contains("without approximation"), "{error}");
    }

    #[test]
    fn branched_outline_is_refused() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb")
            .replace(
                "\n)",
                "\n  (gr_line (start 80 30) (end 90 40) (stroke (width 0.05) (type default)) (layer \"Edge.Cuts\") (uuid \"branch\"))\n)",
            );
        let error = export_dsn(Path::new("board.kicad_pcb"), &source, &rules())
            .unwrap_err()
            .to_string();
        assert!(error.contains("degree"), "{error}");
    }

    #[test]
    fn nonzero_pad_shape_offset_is_refused() {
        let source = include_str!("../tests/fixtures/specctra_two_resistors.kicad_pcb").replacen(
            "(size 0.6 0.5)",
            "(size 0.6 0.5)\n\t\t\t(offset 0.1 0)",
            1,
        );
        let error = export_dsn(Path::new("board.kicad_pcb"), &source, &rules())
            .unwrap_err()
            .to_string();
        assert!(error.contains("shape offset"), "{error}");
    }

    // ─── Creation PR 2b: rounded outlines, more pad shapes, zones ────────────

    const ROUNDED: &str = include_str!("../tests/fixtures/specctra_rounded_board.kicad_pcb");

    fn parse_dsn(text: &str) -> dsn::DsnFile {
        let mut tokenizer = ListTokenizer::new(BufReader::new(Cursor::new(text.as_bytes())));
        dsn::DsnFile::read_dsn(&mut tokenizer).expect("the DSN parses")
    }

    fn export_rounded(source: &str) -> Result<ExportBundle> {
        export_dsn(Path::new("board.kicad_pcb"), source, &rules())
    }

    fn boundary_points(export: &ExportBundle) -> Vec<(f64, f64)> {
        match parse_dsn(&export.dsn).pcb.structure.boundary {
            dsn::Boundary::Path(path) => path.coords.iter().map(|p| (p.x, p.y)).collect(),
            other => panic!("unexpected boundary {other:?}"),
        }
    }

    /// Distance of a DSN point (um, Y up) from the board's true outline: a
    /// 50 x 40 mm rectangle from (80, 30) with 3 mm corner radius, in KiCad mm.
    fn distance_from_rounded_rectangle_mm(point: (f64, f64)) -> f64 {
        let (x, y) = (point.0 / 1000.0, -point.1 / 1000.0);
        let (x0, y0, x1, y1, r): (f64, f64, f64, f64, f64) = (80.0, 30.0, 130.0, 70.0, 3.0);
        // The rectangle shrunk by r is where the corner centres live; the outline
        // is every point r from it.
        let (cx, cy) = (x.clamp(x0 + r, x1 - r), y.clamp(y0 + r, y1 - r));
        (((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r).abs()
    }

    #[test]
    fn a_rounded_outline_is_exported_as_chords_within_the_stated_tolerance() {
        let export = export_rounded(ROUNDED).unwrap();
        let points = boundary_points(&export);
        // Four straight sides and four quarter arcs of 3 mm, each cut into
        // several chords: far more corners than a rectangle.
        assert!(points.len() > 40, "{} points", points.len());
        assert_eq!(points.first(), points.last(), "the loop is closed");
        // Every vertex is on the real outline (1 um for the micrometre grid).
        for point in &points {
            let off = distance_from_rounded_rectangle_mm(*point);
            assert!(off < 0.0011, "{point:?} is {off} mm off the outline");
        }
        // And no chord strays further than the tolerance: its middle is inside
        // by the sagitta, which must stay within ARC_TOLERANCE_MM (plus grid).
        let worst = points
            .windows(2)
            .map(|pair| {
                let mid = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
                distance_from_rounded_rectangle_mm(mid)
            })
            .fold(0.0_f64, f64::max);
        assert!(worst <= ARC_TOLERANCE_MM + 0.0011, "worst chord {worst} mm");
        // It really is rounded: no vertex at the sharp corner.
        assert!(!points.contains(&(80_000.0, -30_000.0)));
        let manifest: serde_json::Value = serde_json::from_str(&export.manifest).unwrap();
        assert!(manifest["supported_profile"]["outline"]
            .as_str()
            .unwrap()
            .contains("within 5 um"));
    }

    #[test]
    fn the_same_outline_in_the_opposite_direction_gives_the_same_loop() {
        let forward = boundary_points(&export_rounded(ROUNDED).unwrap());
        // Swap every Edge.Cuts line and arc's ends; the loop must still close,
        // visiting the same vertices.
        let mut reversed = ROUNDED.to_string();
        for tag in ["gr_line", "gr_arc"] {
            let mut out = String::new();
            let mut rest = reversed.as_str();
            while let Some(at) = rest.find(&format!("({tag}\n")) {
                out.push_str(&rest[..at]);
                let block_end = rest[at..].find("(layer \"Edge.Cuts\")").unwrap() + at;
                let block = &rest[at..block_end];
                let swapped = block
                    .replace("(start ", "(@@ ")
                    .replace("(end ", "(start ")
                    .replace("(@@ ", "(end ");
                out.push_str(&swapped);
                rest = &rest[block_end..];
            }
            out.push_str(rest);
            reversed = out;
        }
        assert_ne!(reversed, ROUNDED);
        let backward = boundary_points(&export_rounded(&reversed).unwrap());
        let sorted = |mut v: Vec<(f64, f64)>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v.dedup();
            v
        };
        assert_eq!(sorted(forward), sorted(backward));
    }

    fn replace_outline(source: &str, outline: &str) -> String {
        let start = source
            .find("\t(gr_arc")
            .unwrap_or_else(|| source.find("\t(gr_line").unwrap());
        let first = source.find("\t(gr_line").map_or(start, |l| l.min(start));
        let end = source.rfind("\t(embedded_fonts no)\n)").unwrap();
        // Drop every Edge.Cuts item, keeping the zones that follow them.
        let zones_at = source[first..end]
            .find("\t(zone")
            .map_or(end, |z| first + z);
        format!("{}{}{}", &source[..first], outline, &source[zones_at..])
    }

    const STROKE: &str = "(stroke (width 0.05) (type default)) (layer \"Edge.Cuts\") (uuid \"55555555-5555-4555-8555-555555555555\")";

    #[test]
    fn a_circle_and_a_rectangle_and_a_polygon_outline_are_accepted() {
        let circle = replace_outline(
            ROUNDED,
            &format!("\t(gr_circle (center 105 50) (end 125 50) {STROKE})\n"),
        );
        let points = boundary_points(&export_rounded(&circle).unwrap());
        assert!(points.len() > 50);
        for (x, y) in &points {
            let radius = ((x / 1000.0 - 105.0).powi(2) + (-y / 1000.0 - 50.0).powi(2)).sqrt();
            assert!((radius - 20.0).abs() < 0.0011, "{radius}");
        }
        let rectangle = replace_outline(
            ROUNDED,
            &format!("\t(gr_rect (start 80 30) (end 130 70) {STROKE})\n"),
        );
        let points = boundary_points(&export_rounded(&rectangle).unwrap());
        assert_eq!(points.len(), 5, "four corners and the closing one");
        let polygon = replace_outline(
            ROUNDED,
            &format!("\t(gr_poly (pts (xy 80 30) (xy 130 30) (xy 130 70) (xy 100 75) (xy 80 70)) {STROKE})\n"),
        );
        assert_eq!(boundary_points(&export_rounded(&polygon).unwrap()).len(), 6);
    }

    #[test]
    fn outlines_the_exporter_cannot_follow_are_still_refused() {
        // A Bezier curve has no chord form here.
        let curve = replace_outline(
            ROUNDED,
            &format!("\t(gr_curve (pts (xy 80 30) (xy 90 20) (xy 120 20) (xy 130 30)) {STROKE})\n"),
        );
        let error = export_rounded(&curve).unwrap_err().to_string();
        assert!(
            error.contains("gr_curve") && error.contains("not a line, arc"),
            "{error}"
        );
        // A circle beside a rectangle is two loops.
        let two = replace_outline(
            ROUNDED,
            &format!("\t(gr_circle (center 105 50) (end 110 50) {STROKE})\n\t(gr_rect (start 80 30) (end 130 70) {STROKE})\n"),
        );
        let error = export_rounded(&two).unwrap_err().to_string();
        assert!(error.contains("more than one loop"), "{error}");
        // An arc in KiCad 6's centre-and-angle form.
        let old = replace_outline(
            ROUNDED,
            &format!("\t(gr_arc (start 100 50) (end 120 50) (angle 90) {STROKE})\n"),
        );
        let error = export_rounded(&old).unwrap_err().to_string();
        assert!(error.contains("old centre-and-angle form"), "{error}");
        // An open chain of arcs and lines.
        let open = ROUNDED.replacen(
            "\t(gr_line\n\t\t(start 80 67)",
            "\t(gr_line\n\t\t(start 80 66)",
            1,
        );
        assert_ne!(open, ROUNDED);
        let error = export_rounded(&open).unwrap_err().to_string();
        assert!(error.contains("degree"), "{error}");
    }

    fn padstacks(export: &ExportBundle) -> Vec<dsn::Padstack> {
        parse_dsn(&export.dsn)
            .pcb
            .library
            .padstacks
            .into_iter()
            .filter(|p| p.name.starts_with("konnect_pad_"))
            .collect()
    }

    #[test]
    fn a_roundrect_pad_is_written_as_its_bounding_rectangle() {
        let export = export_rounded(ROUNDED).unwrap();
        let stacks = padstacks(&export);
        assert_eq!(stacks.len(), 1);
        match &stacks[0].shapes[..] {
            [dsn::Shape::Rect(rect)] => {
                // 0.6 x 0.5 mm, never smaller than the copper.
                assert_eq!((rect.x2 - rect.x1, rect.y2 - rect.y1), (600.0, 500.0));
            }
            other => panic!("{other:?}"),
        }
        let manifest: serde_json::Value = serde_json::from_str(&export.manifest).unwrap();
        assert_eq!(manifest["padstacks"][0]["shape"], "roundrect");
        assert!(manifest["supported_profile"]["pad_shapes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|shape| shape == "roundrect"));
    }

    #[test]
    fn an_oval_pad_is_a_stadium_and_an_equal_sided_oval_is_a_circle() {
        let oval = ROUNDED.replace("smd roundrect", "smd oval");
        let stacks = padstacks(&export_rounded(&oval).unwrap());
        match &stacks[0].shapes[..] {
            [dsn::Shape::Path(path)] => {
                // 0.6 x 0.5: a 0.5 mm wide path 0.1 mm long along X.
                assert_eq!(path.width, 500.0);
                let xs: Vec<f64> = path.coords.iter().map(|p| p.x).collect();
                assert_eq!(xs, [-50.0, 50.0]);
                assert!(path.coords.iter().all(|p| p.y == 0.0));
            }
            other => panic!("{other:?}"),
        }
        let tall = oval.replace("(size 0.6 0.5)", "(size 0.5 0.6)");
        let stacks = padstacks(&export_rounded(&tall).unwrap());
        match &stacks[0].shapes[..] {
            [dsn::Shape::Path(path)] => {
                assert_eq!(path.width, 500.0);
                assert!(path.coords.iter().all(|p| p.x == 0.0));
            }
            other => panic!("{other:?}"),
        }
        let round = oval.replace("(size 0.6 0.5)", "(size 0.5 0.5)");
        let stacks = padstacks(&export_rounded(&round).unwrap());
        assert!(matches!(stacks[0].shapes[..], [dsn::Shape::Circle(_)]));
        // Shapes with no cheap exact form are still refused.
        let trapezoid = ROUNDED.replace("smd roundrect", "smd trapezoid");
        let error = export_rounded(&trapezoid).unwrap_err().to_string();
        assert!(error.contains("trapezoid"), "{error}");
    }

    #[test]
    fn a_pour_and_a_keepout_are_exported_as_a_plane_and_a_keepout() {
        let export = export_rounded(ROUNDED).unwrap();
        let structure = parse_dsn(&export.dsn).pcb.structure;
        assert_eq!(structure.planes.len(), 1);
        let plane = &structure.planes[0];
        assert_eq!(
            (plane.net.as_str(), plane.polygon.layer.as_str()),
            ("GND", "B.Cu")
        );
        // The outline, closed, in DSN micrometres with Y up.
        let corners: Vec<(f64, f64)> = plane.polygon.coords.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(
            corners,
            [
                (85_000.0, -35_000.0),
                (125_000.0, -35_000.0),
                (125_000.0, -65_000.0),
                (85_000.0, -65_000.0),
                (85_000.0, -35_000.0)
            ]
        );
        assert_eq!(structure.keepouts.0.len(), 1);
        let keepout = &structure.keepouts.0[0];
        assert_eq!(keepout.kind, dsn::KeepoutKind::Normal);
        assert!(matches!(&keepout.shape, dsn::Shape::Polygon(p) if p.layer == "F.Cu"));
        let manifest: serde_json::Value = serde_json::from_str(&export.manifest).unwrap();
        let profile = &manifest["supported_profile"];
        assert_eq!(profile["copper_zones"], true);
        assert_eq!(profile["zone_plane_count"], 1);
        assert_eq!(profile["zone_keepout_count"], 1);
        // KiCad's own export of this board writes the same two things.
        let native = include_str!("../tests/fixtures/specctra_rounded_board.native-kicad-10.dsn");
        let native = parse_dsn(native).pcb.structure;
        assert_eq!(native.planes.len(), 1);
        assert_eq!(native.planes[0].net, "GND");
        assert_eq!(native.planes[0].polygon.layer, "B.Cu");
        assert_eq!(native.keepouts.0.len(), 1);
    }

    #[test]
    fn keepouts_forbid_exactly_what_the_rule_area_forbids() {
        let tracks_only = ROUNDED.replace("(vias not_allowed)", "(vias allowed)");
        let structure = parse_dsn(&export_rounded(&tracks_only).unwrap().dsn)
            .pcb
            .structure;
        assert_eq!(structure.keepouts.0[0].kind, dsn::KeepoutKind::Wire);
        let vias_only = ROUNDED.replace("(tracks not_allowed)", "(tracks allowed)");
        let structure = parse_dsn(&export_rounded(&vias_only).unwrap().dsn)
            .pcb
            .structure;
        assert_eq!(structure.keepouts.0[0].kind, dsn::KeepoutKind::Via);
        // A rule area that forbids neither is nothing a router places.
        let neither = vias_only.replace("(vias not_allowed)", "(vias allowed)");
        let export = export_rounded(&neither).unwrap();
        assert!(parse_dsn(&export.dsn).pcb.structure.keepouts.0.is_empty());
        let manifest: serde_json::Value = serde_json::from_str(&export.manifest).unwrap();
        assert_eq!(manifest["supported_profile"]["zone_keepout_count"], 0);
    }

    #[test]
    fn a_pour_on_both_layers_is_one_plane_each_and_a_corner_arc_is_chorded() {
        let both = ROUNDED.replace(
            "(net \"GND\")\n\t\t(layer \"B.Cu\")",
            "(net \"GND\")\n\t\t(layers \"F.Cu\" \"B.Cu\")",
        );
        assert_ne!(both, ROUNDED);
        let structure = parse_dsn(&export_rounded(&both).unwrap().dsn).pcb.structure;
        assert_eq!(structure.planes.len(), 2);
        // A rounded corner in the zone outline.
        let rounded = ROUNDED.replace(
            "(xy 85 35) (xy 125 35) (xy 125 65) (xy 85 65)",
            "(xy 85 35) (xy 122 35) (arc (start 122 35) (mid 124.12132 35.87868) (end 125 38)) (xy 125 65) (xy 85 65)",
        );
        assert_ne!(rounded, ROUNDED);
        let structure = parse_dsn(&export_rounded(&rounded).unwrap().dsn)
            .pcb
            .structure;
        assert!(structure.planes[0].polygon.coords.len() > 8);
    }

    #[test]
    fn zones_the_router_cannot_place_are_refused_not_guessed_at() {
        // A copper zone with no net.
        let no_net = ROUNDED.replacen("(net \"GND\")\n\t\t(layer \"B.Cu\")", "(layer \"B.Cu\")", 1);
        let error = format!("{:#}", export_rounded(&no_net).unwrap_err());
        assert!(error.contains("has no net"), "{error}");
        // A pour on a net no pad is on.
        let orphan = ROUNDED.replacen(
            "(net \"GND\")\n\t\t(layer \"B.Cu\")",
            "(net \"AGND\")\n\t\t(layer \"B.Cu\")",
            1,
        );
        let error = export_rounded(&orphan).unwrap_err().to_string();
        assert!(error.contains("no pad on that net"), "{error}");
        // Two outlines in one zone.
        let two = ROUNDED.replacen(
            "(polygon\n\t\t\t(pts\n\t\t\t\t(xy 85 35)",
            "(polygon\n\t\t\t(pts\n\t\t\t\t(xy 1 1) (xy 2 2) (xy 3 1)\n\t\t\t)\n\t\t)\n\t\t(polygon\n\t\t\t(pts\n\t\t\t\t(xy 85 35)",
            1,
        );
        assert_ne!(two, ROUNDED);
        let error = format!("{:#}", export_rounded(&two).unwrap_err());
        assert!(error.contains("expected one outline polygon"), "{error}");
    }

    #[test]
    fn a_board_with_zones_still_round_trips_through_the_import_plan_and_leaves_them_alone() {
        let ses = include_str!("../tests/fixtures/specctra_rounded_board.freerouting-2.3.0.ses");
        let temp = tempfile::tempdir().unwrap();
        let board_path = temp.path().join("specctra_rounded_board.kicad_pcb");
        std::fs::write(&board_path, ROUNDED).unwrap();
        let export = export_dsn(&board_path, ROUNDED, &native_fixture_rules()).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&export.manifest).unwrap();
        assert_eq!(manifest["supported_profile"]["zone_plane_count"], 1);
        let plan =
            crate::specctra_ses::parse_import_plan(&board_path, ROUNDED, &export.manifest, ses)
                .unwrap();
        // The plan adds tracks and vias; a zone is not among its items.
        assert!(!plan.tracks.is_empty());
        assert!(!format!("{plan:?}").to_lowercase().contains("zone"));
        let adopted = export.clone();

        // The importer refuses a board whose zones are not the ones exported.
        let moved = ROUNDED.replace("(xy 85 35) (xy 125 35)", "(xy 86 35) (xy 125 35)");
        assert_ne!(moved, ROUNDED);
        let error =
            crate::specctra_ses::parse_import_plan(&board_path, &moved, &adopted.manifest, ses)
                .unwrap_err()
                .to_string();
        assert!(
            error.contains("revision") || error.contains("zone"),
            "{error}"
        );
        // A manifest that misstates the zone inventory is refused.
        let mut lie: serde_json::Value = serde_json::from_str(&adopted.manifest).unwrap();
        lie["supported_profile"]["zone_plane_count"] = serde_json::json!(0);
        let error =
            crate::specctra_ses::parse_import_plan(&board_path, ROUNDED, &lie.to_string(), ses)
                .unwrap_err()
                .to_string();
        assert!(error.contains("zone inventory"), "{error}");
        let mut flag: serde_json::Value = serde_json::from_str(&adopted.manifest).unwrap();
        flag["supported_profile"]["copper_zones"] = serde_json::json!(false);
        let error =
            crate::specctra_ses::parse_import_plan(&board_path, ROUNDED, &flag.to_string(), ses)
                .unwrap_err()
                .to_string();
        assert!(error.contains("zone inventory"), "{error}");
    }

    #[test]
    fn kicads_own_export_of_roundrect_pads_is_not_adopted_and_says_why() {
        // KiCad writes a rounded pad as a polygon, which the native-bridge
        // differential check does not model. Adoption refuses it, so the
        // exporter keeps its own DSN (the caller reports the diagnostic).
        let native = include_str!("../tests/fixtures/specctra_rounded_board.native-kicad-10.dsn");
        let baseline = export_dsn(
            Path::new("board.kicad_pcb"),
            ROUNDED,
            &native_fixture_rules(),
        )
        .unwrap();
        let error = format!(
            "{:#}",
            adopt_native_dsn(baseline, native.to_string()).unwrap_err()
        );
        assert!(error.contains("unsupported shape"), "{error}");
    }

    /// Real-engine proof for the rounded board. Needs Java 21 or later and a
    /// Freerouting 2.x JAR: `FREEROUTING_JAR=... cargo test -p konnect-core
    /// freerouting_routes_the_rounded_board -- --ignored`.
    #[test]
    #[ignore = "requires Java 21+ and FREEROUTING_JAR"]
    fn freerouting_routes_the_rounded_board() {
        let jar = std::env::var_os("FREEROUTING_JAR").expect("set FREEROUTING_JAR");
        let temp = tempfile::tempdir().expect("tempdir");
        let board_path = temp.path().join("board.kicad_pcb");
        let dsn_path = temp.path().join("board.dsn");
        let ses_path = temp.path().join("board.ses");
        std::fs::write(&board_path, ROUNDED).unwrap();
        let export = export_dsn(&board_path, ROUNDED, &rules()).unwrap();
        std::fs::write(&dsn_path, &export.dsn).unwrap();
        let output = std::process::Command::new("java")
            .arg("-jar")
            .arg(jar)
            .arg("-de")
            .arg(&dsn_path)
            .arg("-do")
            .arg(&ses_path)
            .arg("-mp")
            .arg("2")
            .output()
            .expect("launch Freerouting");
        assert!(
            output.status.success(),
            "Freerouting refused the DSN:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let ses = std::fs::read_to_string(&ses_path).expect("Freerouting wrote an SES");
        let plan =
            crate::specctra_ses::parse_import_plan(&board_path, ROUNDED, &export.manifest, &ses)
                .expect("the SES passes Konnect's strict import planner");
        assert!(!plan.tracks.is_empty());
    }
}

#[cfg(test)]
#[path = "specctra_layers_tests.rs"]
mod layers_tests;
