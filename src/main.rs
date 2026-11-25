//! Binary entry point of the `ladon` command. The command logic is in `ladon_explorer::entry`.
fn main() -> anyhow::Result<()> {
    ladon_explorer::entry::main_entry()
}
