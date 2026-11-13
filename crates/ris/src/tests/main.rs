use std::collections::BTreeSet;

use clap::CommandFactory;

/// Every command a person or an agent can run, as its subcommand
/// path: `ready`, `phase open`.
fn leaves(command: &clap::Command, path: &[&str], out: &mut BTreeSet<String>) {
    for sub in command.get_subcommands() {
        let mut here = path.to_vec();
        here.push(sub.get_name());
        if sub.has_subcommands() {
            leaves(sub, &here, out);
        } else {
            out.insert(here.join(" "));
        }
    }
}

/// A command added without a type for its `--json` data has no
/// place in the schema; a schema entry for a command that is gone
/// describes nothing. Either fails here.
#[test]
fn the_schema_has_the_data_of_every_command_and_of_no_other() {
    let mut commands = BTreeSet::new();
    leaves(&super::Cli::command(), &[], &mut commands);
    let mut generator = schemars::generate::SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    let described: BTreeSet<String> = ris::schema::commands(&mut generator)
        .into_iter()
        .map(|(name, _)| name.to_owned())
        .collect();
    assert_eq!(described, commands);
}
