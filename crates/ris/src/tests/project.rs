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
