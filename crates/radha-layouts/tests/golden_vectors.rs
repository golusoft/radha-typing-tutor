//! Golden vectors extracted from Golu Expert AI tests (`tests/fixtures`).
//!
//! Every fixture records its Golu source path and sha256; every case is tagged
//! `must_match` (Radha gives Golu's answer) or `intentional_divergence`
//! (Radha differs on purpose: needs a reason, listed in the gap log).
//! `radha_expected` is asserted; `golu_expected` is documentation. All
//! problems are collected and reported together.

mod common;

use std::collections::BTreeSet;

use radha_layouts::krutidev::{expected_round_trip, to_legacy, to_unicode};
use radha_layouts::{Guide, Layer, NextAction, PlanStep};
use serde_json::Value;

use common::{is_sha256, layout, tests_dir, type_keys};

fn layer_name(l: Layer) -> &'static str {
    match l {
        Layer::Base => "base",
        Layer::Shift => "shift",
        Layer::AltGr => "altgr",
        Layer::ShiftAltGr => "shiftaltgr",
    }
}

fn check_source(src: &Value, file: &str, problems: &mut Vec<String>) {
    if src["origin"] != "golu-expert-ai" {
        problems.push(format!("{file}: origin"));
    }
    if !src["path"].as_str().is_some_and(|p| !p.is_empty()) {
        problems.push(format!("{file}: source path"));
    }
    if !src["sha256"].as_str().is_some_and(is_sha256) {
        problems.push(format!("{file}: sha256 must be 64 lowercase hex"));
    }
}

/// The US characters that type a plan, Shift as the shifted character.
fn plan_keys(guide: &Guide) -> String {
    let mut out = String::new();
    for unit in guide.units() {
        let Some(plan) = guide.plan_for(unit) else {
            out.push('?');
            continue;
        };
        for step in &plan.steps {
            match step {
                PlanStep::Key(e) if e.layer == Layer::Shift => out.push(e.key.shifted_char()),
                PlanStep::Key(e) => out.push(e.key.base_char()),
                PlanStep::Space => out.push(' '),
                PlanStep::AltCode(c) => out.push_str(&format!("<{c}>")),
            }
        }
    }
    out
}

fn run_case(case: &Value, problems: &mut Vec<String>, file: &str) {
    let name = case["name"].as_str().unwrap_or("?");
    let id = format!("{file}/{name}");
    let expected = &case["radha_expected"];
    let fail = |problems: &mut Vec<String>, got: String| {
        problems.push(format!("{id}: expected {expected}, got {got}"));
    };
    match case["kind"].as_str().unwrap_or("") {
        "type" => {
            let l = layout(case["layout"].as_str().unwrap_or(""));
            let keys = case["keys"].as_str().unwrap_or("");
            let altgr = case["altgr_after"].as_str().and_then(|s| s.chars().next());
            let (text, preedit) = type_keys(l, keys, altgr);
            if expected.as_str() != Some(text.as_str()) {
                fail(problems, format!("{text:?}"));
            }
            if let Some(p) = case.get("radha_preedit") {
                if p.as_str() != preedit {
                    problems.push(format!("{id}: preedit {preedit:?}, expected {p}"));
                }
            }
        }
        "altgr_keycaps" => {
            let l = layout(case["layout"].as_str().unwrap_or(""));
            let shown: BTreeSet<&str> = radha_layouts::PhysicalKey::all()
                .filter_map(|k| l.table().keycap(k, Layer::AltGr))
                .collect();
            let want: BTreeSet<&str> = expected
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if shown != want {
                fail(problems, format!("{shown:?}"));
            }
        }
        "convert" => {
            let unicode = case["unicode"].as_str().unwrap_or("");
            let c = to_legacy(unicode);
            if expected.as_str() != Some(c.legacy.as_str()) || !c.failures.is_empty() {
                fail(
                    problems,
                    format!("{:?} failures {:?}", c.legacy, c.failures),
                );
            }
            let back = to_unicode(&c.legacy).text;
            if back != expected_round_trip(unicode) {
                problems.push(format!("{id}: round trip gave {back:?}"));
            }
        }
        "convert_refused" => {
            let c = to_legacy(case["unicode"].as_str().unwrap_or(""));
            if expected.as_u64() != Some(c.failures.len() as u64) {
                fail(problems, format!("{} failures", c.failures.len()));
            }
        }
        "hint" => {
            let l = layout(case["layout"].as_str().unwrap_or(""));
            let output = case["output"].as_str().unwrap_or("");
            let guide = Guide::new(l, output);
            let action = if output == " " {
                guide
                    .hint_for(
                        radha_layouts::Keystroke::new(radha_layouts::SPACE, Layer::Base),
                        false,
                    )
                    .map(NextAction::Key)
            } else {
                guide.next_action(0, None, None, false)
            };
            let Some(NextAction::Key(h)) = action else {
                fail(problems, format!("{action:?}"));
                return;
            };
            let got = serde_json::json!({
                "legend": h.legend,
                "layer": layer_name(h.layer),
                "hand": format!("{:?}", h.hand).to_lowercase(),
                "finger": format!("{:?}", h.finger).to_lowercase(),
            });
            if &got != expected {
                fail(problems, got.to_string());
            }
        }
        "plan" => {
            let l = layout(case["layout"].as_str().unwrap_or(""));
            let guide = Guide::new(l, case["unicode"].as_str().unwrap_or(""));
            let got = plan_keys(&guide);
            if expected.as_str() != Some(got.as_str()) {
                fail(problems, format!("{got:?}"));
            }
        }
        other => problems.push(format!("{id}: unknown kind {other:?}")),
    }
}

#[test]
fn every_golden_vector_holds() {
    let dir = tests_dir().join("fixtures");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("golu_"))
        })
        .collect();
    files.sort();
    assert!(files.len() >= 6, "fixtures not found in {dir:?}");
    let mut problems = Vec::new();
    let (mut must, mut diverge) = (0, 0);
    for path in &files {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = std::fs::read_to_string(path).expect("read fixture");
        let fixture: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
        if fixture["schema_version"] != 1 {
            problems.push(format!("{file}: schema_version"));
        }
        check_source(&fixture["source"], &file, &mut problems);
        for extra in fixture["extra_sources"].as_array().into_iter().flatten() {
            check_source(extra, &file, &mut problems);
            if !extra["used_for"].as_str().is_some_and(|s| !s.is_empty()) {
                problems.push(format!("{file}: extra source needs used_for"));
            }
        }
        for case in fixture["cases"].as_array().expect("cases") {
            match case["tag"].as_str() {
                Some("must_match") => must += 1,
                Some("intentional_divergence") => {
                    diverge += 1;
                    if !case["reason"].as_str().is_some_and(|r| !r.is_empty()) {
                        problems.push(format!(
                            "{file}/{}: divergence needs a reason",
                            case["name"]
                        ));
                    }
                }
                other => problems.push(format!("{file}/{}: bad tag {other:?}", case["name"])),
            }
            if case.get("golu_expected").is_none() {
                problems.push(format!("{file}/{}: golu_expected missing", case["name"]));
            }
            run_case(case, &mut problems, &file);
        }
    }
    println!(
        "golden vectors: {} files, {must} must_match, {diverge} intentional_divergence",
        files.len()
    );
    assert!(
        problems.is_empty(),
        "golden vector problems:\n{}",
        problems.join("\n")
    );
}
