//! Explicit bounded computational effort. Geometry and response safety are separate.
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};

/// A JSON integer. A whole number written as a float (`50.0`) satisfies the
/// published `"type":"integer"` schema, so it is read as that integer.
pub(super) fn whole(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|n| n.fract() == 0. && (0.0..=9_007_199_254_740_992.).contains(n))
            .map(|n| n as u64)
    })
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub max_candidates: usize,
    pub max_pair_checks: usize,
    pub max_geometry_work: usize,
    pub max_elapsed_ms: u64,
}

impl Limits {
    // Tiers sit one decade apart on every deterministic counter. A search ends
    // on the geometry-work limit or the candidate limit, whichever its mix of
    // cheap and costly positions reaches first; the pair limit and the elapsed
    // time are safety nets. Deep is also the ceiling of a custom budget.
    pub fn standard() -> Self {
        Self {
            max_candidates: 200_000,
            max_pair_checks: 2_000_000,
            max_geometry_work: 200_000_000,
            max_elapsed_ms: 30_000,
        }
    }
    fn deep() -> Self {
        Self {
            max_candidates: 2_000_000,
            max_pair_checks: 20_000_000,
            max_geometry_work: 2_000_000_000,
            max_elapsed_ms: 120_000,
        }
    }
    fn named(level: &str) -> Result<Self> {
        Ok(match level {
            "quick" => Self {
                max_candidates: 20_000,
                max_pair_checks: 200_000,
                max_geometry_work: 20_000_000,
                max_elapsed_ms: 5_000,
            },
            "standard" => Self::standard(),
            "deep" => Self::deep(),
            _ => bail!("effort must be quick, standard or deep; no unbounded mode"),
        })
    }
    pub fn json(self) -> Value {
        json!({"max_candidates":self.max_candidates,"max_pair_checks":self.max_pair_checks,"max_geometry_work":self.max_geometry_work,"max_elapsed_ms":self.max_elapsed_ms})
    }
    fn custom(value: &Value) -> Result<Self> {
        let map = value.as_object().context("budget must be an object")?;
        ensure!(
            map.keys().all(|k| [
                "max_candidates",
                "max_pair_checks",
                "max_geometry_work",
                "max_elapsed_ms"
            ]
            .contains(&k.as_str())),
            "unknown custom budget field"
        );
        let read = |key: &str, max: u64| -> Result<u64> {
            let n = value
                .get(key)
                .and_then(whole)
                .with_context(|| format!("budget.{key} must be a positive integer"))?;
            ensure!(
                n > 0 && n <= max,
                "budget.{key} outside supported range 1..{max}"
            );
            Ok(n)
        };
        let ceiling = Self::deep();
        Ok(Self {
            max_candidates: read("max_candidates", ceiling.max_candidates as u64)? as usize,
            max_pair_checks: read("max_pair_checks", ceiling.max_pair_checks as u64)? as usize,
            max_geometry_work: read("max_geometry_work", ceiling.max_geometry_work as u64)?
                as usize,
            max_elapsed_ms: read("max_elapsed_ms", ceiling.max_elapsed_ms)?,
        })
    }
}

#[derive(Clone)]
pub(super) struct Effort {
    pub limits: Limits,
    requested: Value,
    level: String,
    source: &'static str,
}
impl Effort {
    pub fn read(args: &Value) -> Result<Self> {
        ensure!(
            !(args.get("effort").is_some() && args.get("budget").is_some()),
            "effort and budget are mutually exclusive"
        );
        if let Some(b) = args.get("budget") {
            return Ok(Self {
                limits: Limits::custom(b)?,
                requested: b.clone(),
                level: "custom".into(),
                source: "custom_budget",
            });
        }
        let level = args
            .get("effort")
            .map(|v| v.as_str().context("effort must be a string"))
            .transpose()?
            .unwrap_or("standard");
        Ok(Self {
            limits: Limits::named(level)?,
            requested: args.get("effort").cloned().unwrap_or(Value::Null),
            level: level.into(),
            source: if args.get("effort").is_some() {
                "explicit_tier"
            } else {
                "backend_default"
            },
        })
    }
    pub fn json(&self) -> Value {
        json!({"requested":self.requested,"effective_level":self.level,"source":self.source,"limits":self.limits.json(),"time_policy":"cooperative search limit; bounded input reading, audit preparation and mandatory final consistency checks can extend total elapsed time"})
    }
}

pub(super) fn budget_schema() -> Value {
    let ceiling = Limits::deep();
    json!({"type":"object","properties":{
    "max_candidates":{"type":"integer","minimum":1,"maximum":ceiling.max_candidates},
    "max_pair_checks":{"type":"integer","minimum":1,"maximum":ceiling.max_pair_checks},
    "max_geometry_work":{"type":"integer","minimum":1,"maximum":ceiling.max_geometry_work},
    "max_elapsed_ms":{"type":"integer","minimum":1,"maximum":ceiling.max_elapsed_ms}
},"required":["max_candidates","max_pair_checks","max_geometry_work","max_elapsed_ms"],"additionalProperties":false})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_refine_custom_limits_reject_missing_zero_noninteger_and_unbounded_fields() {
        for (field, maximum) in [
            ("max_candidates", 2_000_000u64),
            ("max_pair_checks", 20_000_000),
            ("max_geometry_work", 2_000_000_000),
            ("max_elapsed_ms", 120_000),
        ] {
            for bad in [
                json!(0),
                json!(0.0),
                json!(-1),
                json!(-1.0),
                json!(1.5),
                json!(true),
                json!("7"),
                json!(maximum + 1),
                json!((maximum + 1) as f64),
            ] {
                let mut budget = Limits::standard().json();
                budget[field] = bad;
                assert!(Effort::read(&json!({"budget":budget})).is_err(), "{field}");
            }
            let mut budget = Limits::standard().json();
            budget.as_object_mut().unwrap().remove(field);
            assert!(Effort::read(&json!({"budget":budget})).is_err());
            // The stated maximum itself is accepted, and the schema states it.
            let mut budget = Limits::standard().json();
            budget[field] = json!(maximum);
            assert!(Effort::read(&json!({"budget":budget})).is_ok(), "{field}");
            // A whole number written as a float satisfies `"type":"integer"`.
            let mut budget = Limits::standard().json();
            budget[field] = json!(1000.0);
            let read = Effort::read(&json!({"budget":budget})).unwrap();
            assert_eq!(read.limits.json()[field], 1000, "{field}");
            assert_eq!(budget_schema()["properties"][field]["maximum"], maximum);
        }
    }

    /// Each tier raises every limit, a decade at a time on the deterministic
    /// counters, and the deep tier is exactly what a custom budget may reach.
    #[test]
    fn route_refine_tiers_grow_by_decades_and_deep_is_the_custom_ceiling() {
        let tier = |level: &str| Limits::named(level).unwrap();
        let (quick, standard, deep) = (tier("quick"), tier("standard"), tier("deep"));
        for (low, high) in [(quick, standard), (standard, deep)] {
            assert_eq!(high.max_candidates, 10 * low.max_candidates);
            assert_eq!(high.max_pair_checks, 10 * low.max_pair_checks);
            assert_eq!(high.max_geometry_work, 10 * low.max_geometry_work);
            assert!(high.max_elapsed_ms > low.max_elapsed_ms);
        }
        assert_eq!(standard.json(), Limits::standard().json());
        assert_eq!(Limits::custom(&deep.json()).unwrap().json(), deep.json());
        assert!(Limits::named("unbounded").is_err());
    }
}
