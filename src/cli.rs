use clap::{ArgMatches, Command};

pub fn get_matches() -> ArgMatches {
    Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("MPD_HERALD_BUILD_VERSION"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .subcommand(Command::new("authenticate").about("Create last.fm user session"))
        .get_matches()
}
