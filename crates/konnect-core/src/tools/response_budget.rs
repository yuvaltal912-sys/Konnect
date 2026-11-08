//! One way for a tool to keep a long result inside the client's size cap.
//!
//! The client refuses a tool result past roughly 50,000 characters, and a
//! refused result is worse than a short one: the assistant sees nothing and
//! falls back to running `kicad-cli` itself. A tool that can return many items
//! therefore takes `limit` and `offset`, stops at a byte budget even before
//! `limit`, and says so (`truncated`, `next_offset`), so a short list is never
//! mistaken for a small design. Counts and a per-rule tally cover every item,
//! including those not shown. Callers that want everything write it to a file
//! with `output`.

use crate::mcp::protocol::CallToolResult;
use crate::tools::invalid_arg;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Characters of listed items in one response. Other fields add a few hundred.
pub(crate) const BYTE_BUDGET: usize = 40_000;
/// Most items one call may ask for. The byte budget still applies.
pub(crate) const MAX_LIMIT: usize = 10_000;
/// Rules listed in a tally before the rest are folded into `other_rules`.
const MAX_TALLY_ROWS: usize = 40;

/// `limit` and `offset` from the arguments. A present value that is not a whole
/// number in range is refused, never replaced by the default.
pub(crate) fn window(args: &Value, default_limit: usize) -> Result<(usize, usize), CallToolResult> {
    let read = |field: &str, default: usize, min: usize| -> Result<usize, CallToolResult> {
        match &args[field] {
            Value::Null => Ok(default),
            value => {
                let number = value
                    .as_u64()
                    .ok_or_else(|| invalid_arg(field, "must be a non-negative whole number"))?;
                let number =
                    usize::try_from(number).map_err(|_| invalid_arg(field, "is out of range"))?;
                if number < min || (field == "limit" && number > MAX_LIMIT) {
                    return Err(invalid_arg(
                        field,
                        &if field == "limit" {
                            format!("must be between {min} and {MAX_LIMIT}")
                        } else {
                            format!("must be at least {min}")
                        },
                    ));
                }
                Ok(number)
            }
        }
    };
    Ok((read("offset", 0, 0)?, read("limit", default_limit, 1)?))
}

/// The items of one response and where they sit in the whole list.
pub(crate) struct Page {
    pub items: Vec<Value>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    /// More items follow the last one shown.
    pub truncated: bool,
    /// The byte budget, not `limit`, ended the page.
    pub byte_capped: bool,
}

/// Take items from `offset` while there are fewer than `limit` and the
/// serialized items fit `budget`. A page always holds one item when any remain,
/// so a single huge item cannot stall paging.
pub(crate) fn page(all: &[Value], offset: usize, limit: usize, budget: usize) -> Page {
    let mut items = Vec::new();
    let mut used = 0usize;
    let mut byte_capped = false;
    for item in all.iter().skip(offset).take(limit) {
        let cost = cost(item);
        if !items.is_empty() && used + cost > budget {
            byte_capped = true;
            break;
        }
        used += cost;
        items.push(item.clone());
    }
    let truncated = offset + items.len() < all.len();
    Page {
        items,
        total: all.len(),
        offset,
        limit,
        truncated,
        byte_capped,
    }
}

/// Characters one item costs in a response, with its separator.
fn cost(item: &Value) -> usize {
    serde_json::to_string(item).map_or(0, |text| text.len()) + 1
}

impl Page {
    /// Characters the shown items cost, as [`page`] counted them.
    pub(crate) fn used(&self) -> usize {
        self.items.iter().map(cost).sum()
    }

    /// Replace the hint [`Page::describe`] wrote, for a tool whose next step
    /// is not `severity` or `output`. Nothing is written for a whole list.
    pub(crate) fn hint(&self, fields: &mut Map<String, Value>, advice: &str) {
        if self.truncated {
            fields.insert(
                "hint".into(),
                json!(format!(
                    "Showing {} of {}. {advice}",
                    self.items.len(),
                    self.total
                )),
            );
        }
    }

    /// The fields that tell a reader what the page is: add these beside the items.
    pub(crate) fn describe(&self, fields: &mut Map<String, Value>) {
        fields.insert("shown".into(), json!(self.items.len()));
        fields.insert("offset".into(), json!(self.offset));
        fields.insert("limit".into(), json!(self.limit));
        fields.insert("truncated".into(), json!(self.truncated));
        if self.truncated {
            let next = self.offset + self.items.len();
            fields.insert("next_offset".into(), json!(next));
            if self.byte_capped {
                fields.insert("byte_capped".into(), json!(true));
            }
            fields.insert(
                "hint".into(),
                json!(format!(
                    "Showing {} of {}. Pass offset {next} for the next page, narrow with severity, \
                     or pass output to write the full report to a file.",
                    self.items.len(),
                    self.total
                )),
            );
        }
    }
}

/// A list result: `count` is the whole list, `key` holds one page of it, and
/// the paging fields say what is left. For the list tools, whose items need no
/// tally.
pub(crate) fn list_response(key: &str, all: &[Value], offset: usize, limit: usize) -> Value {
    let page = page(all, offset, limit, BYTE_BUDGET);
    let mut fields = Map::new();
    fields.insert("count".into(), json!(all.len()));
    fields.insert(key.into(), json!(page.items));
    page.describe(&mut fields);
    // The hint offers `severity` and `output`, which a list tool lacks.
    if page.truncated {
        fields.insert(
            "hint".into(),
            json!(format!(
                "Showing {} of {}. Pass offset {} for the next page.",
                page.items.len(),
                all.len(),
                page.offset + page.items.len()
            )),
        );
    }
    Value::Object(fields)
}

/// Cut each named list in `body` to what fits in `budget`, in the order given,
/// and say which were cut. For results a second call cannot page, such as a
/// batch edit that already happened: the counts beside the lists stay whole,
/// `truncated_lists` gives `shown` and `total` for each list that was cut, and
/// `hint` says where the rest is. A result that fits is left exactly as it
/// was. Returns the characters the kept lists cost.
pub(crate) fn fit_lists(
    body: &mut Map<String, Value>,
    keys: &[&str],
    budget: usize,
    advice: &str,
) -> usize {
    let mut used = 0usize;
    let mut cut = Map::new();
    for key in keys {
        let Some(Value::Array(all)) = body.get(*key) else {
            continue;
        };
        let page = page(all, 0, all.len(), budget.saturating_sub(used));
        used += page.used();
        if page.truncated {
            cut.insert(
                (*key).into(),
                json!({ "shown": page.items.len(), "total": page.total }),
            );
            body.insert((*key).into(), Value::Array(page.items));
        }
    }
    if !cut.is_empty() {
        let lists: Vec<&str> = cut.keys().map(String::as_str).collect();
        let note = format!("Cut to fit the response: {}. {advice}", lists.join(", "));
        body.insert("truncated_lists".into(), Value::Object(cut));
        body.entry("hint").or_insert(json!(note));
    }
    used
}

/// [`fit_lists`] on a whole JSON result with the full [`BYTE_BUDGET`].
pub(crate) fn fit_value(body: &mut Value, keys: &[&str], advice: &str) {
    if let Some(fields) = body.as_object_mut() {
        fit_lists(fields, keys, BYTE_BUDGET, advice);
    }
}

/// How many items each `(rule, severity)` has, most first, over every item and
/// not only those on the page. Past [`MAX_TALLY_ROWS`] groups the rest are
/// counted in one trailing row so the counts still add up.
pub(crate) fn tally(items: &[Value], rule_key: &str) -> Vec<Value> {
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for item in items {
        let rule = item[rule_key].as_str().unwrap_or("").to_owned();
        let severity = item["severity"].as_str().unwrap_or("").to_owned();
        *counts.entry((rule, severity)).or_default() += 1;
    }
    let mut rows: Vec<((String, String), usize)> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let rest: usize = rows.iter().skip(MAX_TALLY_ROWS).map(|row| row.1).sum();
    let mut out: Vec<Value> = rows
        .into_iter()
        .take(MAX_TALLY_ROWS)
        .map(|((rule, severity), count)| json!({ "rule": rule, "severity": severity, "count": count }))
        .collect();
    if rest > 0 {
        out.push(json!({ "other_rules": true, "count": rest }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(count: usize, rule: impl Fn(usize) -> String) -> Vec<Value> {
        (0..count)
            .map(|i| json!({ "rule": rule(i), "severity": if i.is_multiple_of(2) { "error" } else { "warning" }, "n": i }))
            .collect()
    }

    #[test]
    fn a_window_defaults_and_refuses_what_it_cannot_read() {
        assert_eq!(window(&json!({}), 50).unwrap(), (0, 50));
        assert_eq!(
            window(&json!({ "limit": 7, "offset": 3 }), 50).unwrap(),
            (3, 7)
        );
        for bad in [
            json!({ "limit": 0 }),
            json!({ "limit": 10_001 }),
            json!({ "limit": -1 }),
            json!({ "limit": 1.5 }),
            json!({ "limit": "9" }),
            json!({ "offset": -1 }),
            json!({ "offset": "x" }),
        ] {
            let refusal = window(&bad, 50).unwrap_err();
            assert!(refusal.is_error, "{bad}");
        }
    }

    #[test]
    fn a_page_stops_at_the_limit_and_says_where_to_resume() {
        let all = items(10, |i| format!("r{i}"));
        let first = page(&all, 0, 4, BYTE_BUDGET);
        assert_eq!(
            (first.items.len(), first.total, first.truncated),
            (4, 10, true)
        );
        let last = page(&all, 8, 4, BYTE_BUDGET);
        assert_eq!((last.items.len(), last.truncated), (2, false));
        let mut fields = Map::new();
        first.describe(&mut fields);
        assert_eq!(fields["next_offset"], 4);
        assert!(fields["hint"].as_str().unwrap().contains("offset 4"));
        let mut fields = Map::new();
        last.describe(&mut fields);
        assert!(fields.get("next_offset").is_none() && fields.get("hint").is_none());
        assert_eq!(fields["truncated"], false);
    }

    #[test]
    fn the_byte_budget_ends_a_page_before_the_limit() {
        let all = items(500, |i| format!("a_rather_long_rule_name_number_{i:04}"));
        let page = page(&all, 0, 500, 2_000);
        assert!(page.byte_capped && page.truncated);
        let bytes: usize = page.items.iter().map(|i| i.to_string().len() + 1).sum();
        assert!(
            bytes <= 2_000 && page.items.len() > 5 && page.items.len() < 500,
            "{bytes}"
        );
        let mut fields = Map::new();
        page.describe(&mut fields);
        assert_eq!(fields["byte_capped"], true);
    }

    #[test]
    fn an_item_bigger_than_the_budget_still_gets_a_page_of_one() {
        let all = vec![json!({ "text": "x".repeat(5_000) }), json!({ "text": "y" })];
        let first = page(&all, 0, 10, 100);
        assert_eq!(first.items.len(), 1);
        assert!(first.truncated);
        let second = page(&all, 1, 10, 100);
        assert_eq!((second.items.len(), second.truncated), (1, false));
    }

    #[test]
    fn fitted_lists_share_one_budget_in_priority_order() {
        let mut body = Map::new();
        body.insert("failures".into(), json!(items(3, |i| format!("f{i}"))));
        body.insert("placed".into(), json!(items(500, |i| format!("p{i:04}"))));
        body.insert("placed_count".into(), json!(500));
        let used = fit_lists(&mut body, &["failures", "placed"], 2_000, "Read the file.");
        assert!(used <= 2_000, "{used}");
        assert_eq!(
            body["failures"].as_array().unwrap().len(),
            3,
            "first list kept whole"
        );
        let shown = body["placed"].as_array().unwrap().len();
        assert!(shown > 0 && shown < 500);
        assert_eq!(body["placed_count"], 500, "counts are not touched");
        assert_eq!(body["truncated_lists"]["placed"]["total"], 500);
        assert_eq!(body["truncated_lists"]["placed"]["shown"], shown);
        assert!(body["truncated_lists"].get("failures").is_none());
        assert!(body["hint"]
            .as_str()
            .unwrap()
            .contains("placed. Read the file."));
    }

    #[test]
    fn fitted_lists_leave_a_short_result_unchanged() {
        let mut body = Map::new();
        body.insert("added".into(), json!(items(5, |i| format!("a{i}"))));
        body.insert("missing".into(), json!("not a list"));
        let before = body.clone();
        fit_lists(&mut body, &["added", "absent", "missing"], BYTE_BUDGET, "x");
        assert_eq!(body, before);
    }

    #[test]
    fn the_tally_counts_every_item_and_folds_a_long_tail() {
        let all = items(100, |i| format!("rule_{:03}", i % 60));
        let rows = tally(&all, "rule");
        let counted: u64 = rows.iter().map(|r| r["count"].as_u64().unwrap()).sum();
        assert_eq!(counted, 100, "the rows add up to every item");
        assert_eq!(rows.len(), MAX_TALLY_ROWS + 1);
        assert_eq!(rows.last().unwrap()["other_rules"], true);
        // Most frequent first: rules 0..40 occur twice, the rest once.
        assert_eq!(rows[0]["count"], 2);
    }
}
