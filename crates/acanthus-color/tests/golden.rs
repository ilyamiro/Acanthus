use std::collections::BTreeMap;

use acanthus_color::color::Argb;
use acanthus_color::generate::{SchemeType, hex, roles, scheme};
use serde_json::Value;

type Golden = BTreeMap<String, BTreeMap<String, Value>>;

fn parse_source(hex: &str) -> Argb {
    hex.parse().expect("valid hex")
}

fn check(golden: &Golden) -> usize {
    let mut compared = 0;
    for (source, per_type) in golden {
        for (type_name, expected) in per_type {
            let kind = SchemeType::parse(&format!("scheme-{type_name}")).unwrap();
            for (mode, dark) in [("dark", true), ("light", false)] {
                let produced = roles(scheme(parse_source(source), kind, dark, None));
                let wanted = expected[mode].as_object().unwrap();
                for (role, color) in &produced {
                    if let Some(want) = wanted.get(role) {
                        assert_eq!(hex(*color), want.as_str().unwrap(), "{source} {type_name} {mode} {role}");
                        compared += 1;
                    }
                }
                assert!(
                    wanted.keys().filter(|k| *k != "source_color").all(|k| produced.iter().any(|(r, _)| r == k)),
                    "{source} {type_name}: missing role"
                );
            }
        }
    }
    compared
}

#[test]
fn committed_golden_matches_matugen_for_every_scheme_type() {
    let golden: Golden = serde_json::from_str(include_str!("golden_schemes.json")).unwrap();
    assert_eq!(golden.len(), 4);
    assert!(check(&golden) >= 4 * 9 * 2 * 49);
}

#[test]
fn external_golden_file_matches_when_provided() {
    let Ok(path) = std::env::var("SERP_COLOR_GOLDEN") else { return };
    let golden: Golden = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let compared = check(&golden);
    assert!(compared > 0);
}
