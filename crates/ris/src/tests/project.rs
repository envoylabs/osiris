use super::*;
use crate::config::Role;

/// Each role's refusal names its own next step: only an auditor is
/// pointed at the review queue, only an advisor at what it may do.
#[test]
fn a_role_refusal_names_that_role_s_next_step() {
    for &role in Role::ALL {
        let text = CliError::RoleForbids {
            seat: "kite".into(),
            role,
            what: "claim an item",
        }
        .to_string();
        assert!(
            text.contains(&format!("is an {role}")) || text.contains(&format!("is a {role}")),
            "{text}"
        );
        assert!(
            !text.contains("an lead") && !text.contains("an builder"),
            "{text}"
        );
        assert_eq!(text.contains("ris audit"), role == Role::Auditor, "{text}");
        assert_eq!(
            text.contains("ris item add"),
            role == Role::Advisor,
            "{text}"
        );
    }
}

/// The rules `ris init` writes keep the rule that a test does not
/// depend on the machine it runs on, in their Tests section.
#[test]
fn the_rules_say_a_test_does_not_depend_on_its_machine() {
    let tests = RULES
        .split("\n## ")
        .find(|section| section.starts_with("Tests\n"))
        .expect("the rules have a Tests section");
    let said = tests.split_whitespace().collect::<Vec<_>>().join(" ");
    for part in [
        "A test never depends on the machine it runs on",
        "no path, clock speed, system tool or configuration that differs between systems",
        "A test that needs one sets it up itself",
    ] {
        assert!(said.contains(part), "missing {part:?} in:\n{tests}");
    }
}
