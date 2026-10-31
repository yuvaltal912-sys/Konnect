//! KiCad net names as they appear in a SPICE netlist.
//!
//! A port of `NETLIST_EXPORTER_SPICE::ConvertToSpiceMarkup`, needed because the
//! SPICE export has no pin-to-net map. Retire it if KiCad exports that map.
//!
//! The rules differ within 10.0: `/0` and `/gnd` stay ordinary nodes at 10.0.1
//! and become ground in later builds. So [`spice_candidates`] offers both forms
//! and the caller keeps whichever the netlist in hand actually contains.

/// Every name this net could have in a SPICE netlist, most likely first.
pub(crate) fn spice_candidates(kicad_name: &str) -> Vec<String> {
    let base = base_conversion(kicad_name);
    let mut names = vec![base.clone()];
    if base.ends_with("/0") && !base.ends_with("//0") {
        names.push("0".to_string());
    }
    if base.eq_ignore_ascii_case("/gnd") {
        names.push(base[1..].to_string());
    }
    names
}

fn base_conversion(kicad_name: &str) -> String {
    let mut name = flatten_markup(&unescape(kicad_name));
    name = name.replace(['%', '(', ')', ',', '[', ']', '<', '>', '~', ' '], "_");
    // ngspice reads `//` as a comment, so KiCad names the root explicitly.
    if name.starts_with("//") {
        name = name.replacen("//", "/root/", 1);
    }
    name
}

fn unescape(name: &str) -> String {
    [
        ("{slash}", "/"),
        ("{backslash}", "\\"),
        ("{colon}", ":"),
        ("{dblquote}", "\""),
        ("{lt}", "<"),
        ("{gt}", ">"),
    ]
    .iter()
    .fold(name.to_string(), |name, (escaped, plain)| {
        name.replace(escaped, plain)
    })
}

/// `~{CLK}` stays distinct from `CLK`, but `V_{OUT}` is only a pretty `VOUT`.
fn flatten_markup(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(open) = rest.find('{') {
        let marker = rest[..open].chars().last();
        let Some(close) = rest[open..].find('}').map(|end| open + end) else {
            break;
        };
        match marker {
            Some('~') => {
                out.push_str(&rest[..open]);
            }
            Some('_') | Some('^') => {
                out.push_str(&rest[..open - 1]);
            }
            _ => {
                out.push_str(&rest[..=close]);
                rest = &rest[close + 1..];
                continue;
            }
        }
        out.push_str(&flatten_markup(&rest[open + 1..close]));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(name: &str) -> String {
        spice_candidates(name).remove(0)
    }

    #[test]
    fn plain_names_pass_through() {
        assert_eq!(first("/rect_out"), "/rect_out");
        assert_eq!(first("GND"), "GND");
        assert_eq!(first("+5V"), "+5V");
    }

    #[test]
    fn disallowed_characters_become_underscores() {
        assert_eq!(first("Net-(BT1-+)"), "Net-_BT1-+_");
        assert_eq!(first("/D[0]"), "/D_0_");
        assert_eq!(first("/x y"), "/x_y");
        assert_eq!(first("/100%"), "/100_");
        assert_eq!(first("/a,b"), "/a_b");
        assert_eq!(first("/<n>"), "/_n_");
    }

    #[test]
    fn subscripts_and_superscripts_flatten_and_overbars_stay_distinct() {
        assert_eq!(first("/V_{OUT}"), "/VOUT");
        assert_eq!(first("/VOUT^{2}"), "/VOUT2");
        assert_eq!(first("/~{RST}"), "/_RST");
        assert_eq!(first("/~{A}B"), "/_AB");
    }

    #[test]
    fn escapes_are_undone_before_conversion() {
        assert_eq!(first("/a{slash}b"), "/a/b");
    }

    #[test]
    fn a_root_label_starting_with_a_slash_avoids_the_comment_marker() {
        assert_eq!(first("//foo"), "/root/foo");
    }

    #[test]
    fn distinct_nets_can_collide() {
        assert_eq!(first("/A(1)"), first("/A[1]"));
    }

    #[test]
    fn local_ground_names_offer_both_kicad_behaviours() {
        assert_eq!(spice_candidates("/0"), ["/0", "0"]);
        assert_eq!(spice_candidates("/gnd"), ["/gnd", "gnd"]);
        assert_eq!(spice_candidates("/GND"), ["/GND", "GND"]);
        assert_eq!(spice_candidates("/sub/0"), ["/sub/0", "0"]);
        assert_eq!(spice_candidates("//0"), ["/root/0", "0"]);
    }

    #[test]
    fn unbalanced_markup_is_left_as_it_is() {
        assert_eq!(first("/a~{b"), "/a_{b");
        assert_eq!(first("{plain}"), "{plain}");
    }
}
